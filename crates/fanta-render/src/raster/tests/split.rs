use super::*;
use fanta_doc::{
    AutoLayout, BitmapNode, Doc, Operation, TextNode, TextStyleRun, UnitInterval, VectorNode,
};
use skia_safe::{AlphaType, ColorType, ImageInfo, Surface};

const WIDTH: u32 = 192;
const HEIGHT: u32 = 160;

fn add(doc: &mut Doc, parent: Option<NodeId>, data: NodeData, x: f64, y: f64) -> NodeId {
    let mut node = CanvasNode::new(data);
    node.parent = parent;
    node.index = doc.scene.next_child_index(parent);
    node.transform = Transform2D::translation(x, y);
    let id = node.id;
    doc.apply(Operation::create_node(node))
        .expect("fixture node");
    id
}

fn rectangle(color: Color) -> NodeData {
    NodeData::Vector(VectorNode::rect_solid(0.0, 0.0, 55.0, 46.0, color))
}

fn fixture(reverse: bool, page_color: Color) -> (Doc, NodeId, NodeId, NodeId) {
    let mut doc = Doc::new();
    let page = add(
        &mut doc,
        None,
        NodeData::Group(GroupNode {
            background: Some(Fill::solid(page_color)),
            ..Default::default()
        }),
        0.0,
        0.0,
    );
    add(
        &mut doc,
        Some(page),
        rectangle(Color::rgb(20, 80, 210)),
        -65.0,
        -35.0,
    );
    let parent = add(
        &mut doc,
        Some(page),
        NodeData::Group(GroupNode {
            auto_layout: Some(AutoLayout {
                reverse_z: reverse,
                ..Default::default()
            }),
            ..Default::default()
        }),
        5.5,
        -3.25,
    );
    add(
        &mut doc,
        Some(parent),
        rectangle(Color::rgba(255, 190, 20, 170)),
        -35.0,
        -25.0,
    );
    let moving = add(
        &mut doc,
        Some(parent),
        rectangle(Color::rgba(220, 30, 90, 200)),
        -10.0,
        -12.0,
    );
    add(
        &mut doc,
        Some(parent),
        rectangle(Color::rgba(20, 230, 110, 140)),
        15.0,
        5.0,
    );
    let mut text = TextNode::new("Split text", 88.0, 32.0);
    text.style.color = Color::rgba(250, 250, 250, 210);
    text.style.size_px = 18.0;
    text.style_runs.push(TextStyleRun {
        start: 6,
        end: 10,
        style: fanta_doc::TextStyle {
            weight: 700,
            color: Color::rgb(20, 15, 80),
            ..text.style.clone()
        },
    });
    add(&mut doc, Some(parent), NodeData::Text(text), -30.0, 8.5);
    add(
        &mut doc,
        Some(page),
        rectangle(Color::rgba(30, 40, 220, 100)),
        -20.0,
        28.0,
    );
    (doc, page, parent, moving)
}

fn surface() -> Surface {
    skia_safe::surfaces::raster_n32_premul((WIDTH as i32, HEIGHT as i32)).expect("test surface")
}

fn pixels(surface: &mut Surface) -> Vec<u8> {
    let info = ImageInfo::new(
        (WIDTH as i32, HEIGHT as i32),
        ColorType::RGBA8888,
        AlphaType::Premul,
        None,
    );
    let mut pixels = vec![0; WIDTH as usize * HEIGHT as usize * 4];
    assert!(surface.read_pixels(&info, &mut pixels, WIDTH as usize * 4, (0, 0)));
    pixels
}

fn assert_parity(
    doc: &Doc,
    page: NodeId,
    moving: NodeId,
    viewport: &Viewport,
    resolver: Option<Arc<dyn AssetResolver>>,
) -> Vec<u8> {
    assert_parity_at_scale(doc, page, moving, viewport, resolver, 1.0)
}

fn assert_parity_at_scale(
    doc: &Doc,
    page: NodeId,
    moving: NodeId,
    viewport: &Viewport,
    resolver: Option<Arc<dyn AssetResolver>>,
    display_scale: f64,
) -> Vec<u8> {
    assert_parity_with_inputs(
        doc,
        page,
        moving,
        viewport,
        resolver,
        display_scale,
        &RenderInputs::empty(),
    )
}

fn assert_parity_with_inputs(
    doc: &Doc,
    page: NodeId,
    moving: NodeId,
    viewport: &Viewport,
    resolver: Option<Arc<dyn AssetResolver>>,
    display_scale: f64,
    inputs: &RenderInputs,
) -> Vec<u8> {
    let before = serde_json::to_value(doc).expect("before scene");
    let spec = SplitSpec::prepare(&doc.scene, page, moving, inputs, resolver.as_deref())
        .expect("eligible split");
    let mut renderer = RasterRenderer::new(WIDTH, HEIGHT).expect("renderer");
    renderer.display_scale = display_scale;
    if let Some(resolver) = resolver {
        renderer.set_asset_resolver(resolver);
    }
    let mut expected = surface();
    let metrics = renderer.render_to_canvas(
        expected.canvas(),
        WIDTH,
        HEIGHT,
        &doc.scene,
        viewport,
        Some(page),
        inputs,
    );
    assert!(!metrics.incomplete_artwork && !metrics.effect_failed);
    let mut actual = surface();
    actual.canvas().clear(skia_safe::Color::TRANSPARENT);
    for phase in [SplitPhase::Below, SplitPhase::Middle, SplitPhase::Above] {
        let metrics = if spec.requires_ordered_paint() && phase != SplitPhase::Below {
            let mut recorder = skia_safe::PictureRecorder::new();
            let recording = recorder
                .begin_recording(skia_safe::Rect::from_wh(WIDTH as f32, HEIGHT as f32), None);
            let metrics = renderer
                .paint_split_to_canvas(
                    recording, WIDTH, HEIGHT, &doc.scene, viewport, inputs, &spec, phase,
                )
                .expect("ordered phase");
            let picture = recorder.finish_recording_as_picture(None).expect("picture");
            let matrix = actual.canvas().local_to_device();
            let clip = actual.canvas().device_clip_bounds();
            let saves = actual.canvas().save_count();
            actual.canvas().draw_picture(picture, None, None);
            assert_eq!(actual.canvas().local_to_device(), matrix);
            assert_eq!(actual.canvas().device_clip_bounds(), clip);
            assert_eq!(actual.canvas().save_count(), saves);
            metrics
        } else {
            let mut layer = surface();
            let metrics = if spec.requires_ordered_paint() {
                renderer.paint_split_to_canvas(
                    layer.canvas(),
                    WIDTH,
                    HEIGHT,
                    &doc.scene,
                    viewport,
                    inputs,
                    &spec,
                    phase,
                )
            } else {
                renderer.render_split_to_canvas(
                    layer.canvas(),
                    WIDTH,
                    HEIGHT,
                    &doc.scene,
                    viewport,
                    inputs,
                    &spec,
                    phase,
                )
            }
            .expect("phase");
            actual
                .canvas()
                .draw_image(layer.image_snapshot(), (0, 0), None);
            metrics
        };
        assert!(!metrics.incomplete_artwork && !metrics.effect_failed);
        assert_eq!(
            (metrics.layer_cache_hits, metrics.layer_cache_misses),
            (0, 0)
        );
    }
    let expected_pixels = pixels(&mut expected);
    let actual_pixels = pixels(&mut actual);
    let differences: Vec<_> = expected_pixels
        .iter()
        .zip(&actual_pixels)
        .map(|(left, right)| left.abs_diff(*right))
        .collect();
    let maximum = differences.iter().copied().max().expect("nonempty image");
    if maximum > 2
        && let Some(directory) = std::env::var_os("FANTA_SPLIT_FAILURE_DIR")
    {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).expect("failure directory");
        std::fs::write(directory.join("expected.rgba"), &expected_pixels).expect("expected pixels");
        std::fs::write(directory.join("actual.rgba"), &actual_pixels).expect("actual pixels");
        std::fs::write(
            directory.join("scene.json"),
            serde_json::to_vec_pretty(doc).expect("failure scene"),
        )
        .expect("failure scene file");
        std::fs::write(
            directory.join("render.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "page": page, "moving": moving, "center": viewport.center,
                "zoom": viewport.zoom, "display_scale": display_scale,
                "width": WIDTH, "height": HEIGHT, "maximum": maximum,
            }))
            .expect("render parameters"),
        )
        .expect("render parameter file");
    }
    assert!(
        maximum <= 2,
        "max channel difference {maximum}; changed channels {}",
        differences
            .iter()
            .filter(|difference| **difference != 0)
            .count()
    );
    let mut normal_after = surface();
    renderer.render_to_canvas(
        normal_after.canvas(),
        WIDTH,
        HEIGHT,
        &doc.scene,
        viewport,
        Some(page),
        inputs,
    );
    assert_eq!(
        pixels(&mut normal_after),
        expected_pixels,
        "split must not poison normal rendering caches"
    );
    assert_eq!(
        serde_json::to_value(doc).expect("after scene"),
        before,
        "render must not mutate authored data/history"
    );
    actual_pixels
}

#[test]
fn split_phases_match_complete_pixels_for_paint_order_backgrounds_and_transforms() {
    for reverse in [false, true] {
        for background in [
            Color::rgb(35, 42, 55),
            Color::rgba(40, 80, 120, 80),
            Color::rgba(0, 0, 0, 0),
        ] {
            let (mut doc, page, parent, moving) = fixture(reverse, background);
            doc.scene
                .set_transform(
                    parent,
                    Transform2D::rotation(0.23).then(&Transform2D::translation(4.25, -3.5)),
                )
                .expect("rotate");
            for viewport in [
                Viewport {
                    center: [0.0, 0.0],
                    zoom: 1.0,
                },
                Viewport {
                    center: [22.25, -13.5],
                    zoom: 0.73,
                },
                Viewport {
                    center: [200.0, 200.0],
                    zoom: 2.0,
                },
            ] {
                assert_parity(&doc, page, moving, &viewport, None);
            }
        }
    }
}

#[test]
fn split_own_normal_effects_and_clipped_subtree_match_full_render() {
    for effect in 0..4 {
        let (mut doc, page, _, moving) = fixture(false, Color::rgb(80, 90, 110));
        let node = doc.scene.get_mut(moving).expect("moving");
        match effect {
            0 => node.opacity = UnitInterval::new(0.43),
            1 => node.effects.push(Shadow {
                kind: ShadowKind::Drop,
                color: Color::rgba(0, 0, 0, 180),
                blur: 5.0,
                spread: 0.0,
                offset: [4.0, 3.0],
                show_behind_node: false,
            }),
            2 => node.blurs.push(Blur::layer(3.0)),
            _ => {
                node.data = NodeData::Group(GroupNode {
                    clip_size: Some([55.0, 46.0]),
                    corner_radius: Some(9.0),
                    background: Some(Fill::solid(Color::rgb(220, 30, 90))),
                    ..Default::default()
                });
                add(
                    &mut doc,
                    Some(moving),
                    rectangle(Color::rgb(30, 210, 110)),
                    30.0,
                    18.0,
                );
            }
        }
        assert_parity(
            &doc,
            page,
            moving,
            &Viewport {
                center: [0.0, 0.0],
                zoom: 1.0,
            },
            None,
        );
    }
}

#[test]
fn split_repeated_moves_and_undo_match_fresh_render() {
    let (mut doc, page, _, moving) = fixture(true, Color::WHITE);
    let viewport = Viewport {
        center: [0.0, 0.0],
        zoom: 1.25,
    };
    let initial = doc.scene.get(moving).expect("moving").transform;
    let baseline = assert_parity(&doc, page, moving, &viewport, None);
    for step in 1..=12 {
        let old = doc.scene.get(moving).expect("moving").transform;
        let new = initial.then(&Transform2D::translation(
            step as f64 * 0.7,
            step as f64 * -0.25,
        ));
        doc.apply(Operation::SetTransform {
            id: moving,
            old,
            new,
        })
        .expect("move");
        assert_parity(&doc, page, moving, &viewport, None);
    }
    for _ in 0..12 {
        doc.undo().expect("undo");
    }
    assert_eq!(doc.scene.get(moving).expect("moving").transform, initial);
    assert_eq!(assert_parity(&doc, page, moving, &viewport, None), baseline);
}

#[test]
fn split_rejects_unsafe_ancestors_and_paint_dependencies() {
    for case in 0..11 {
        let (mut doc, page, parent, moving) = fixture(false, Color::WHITE);
        let node = doc
            .scene
            .get_mut(if case < 5 { parent } else { moving })
            .expect("test node");
        match case {
            0 => node.opacity = UnitInterval::new(0.5),
            1 => node.flags.insert(NodeFlags::ISOLATED_BLEND),
            2 => {
                node.data.as_group_mut().expect("group").background =
                    Some(Fill::solid(Color::WHITE))
            }
            3 => node.blurs.push(Blur::layer(3.0)),
            4 => node.effects.push(Shadow {
                kind: ShadowKind::Inner,
                color: Color::BLACK,
                blur: 4.0,
                spread: 0.0,
                offset: [1.0, 1.0],
                show_behind_node: false,
            }),
            5 => node.blend_mode = BlendMode::Multiply,
            6 => node.blurs.push(Blur::background(4.0)),
            7 => node.is_mask = true,
            8 => node.transform = Transform2D::translation(f64::NAN, 0.0),
            9 => node.data = NodeData::Boolean(Default::default()),
            _ => {
                node.data.as_vector_mut().expect("vector").fills[0] = Fill::Solid {
                    color: Color::WHITE,
                    blend: BlendMode::Screen,
                }
            }
        }
        assert!(
            SplitSpec::prepare(&doc.scene, page, moving, &RenderInputs::empty(), None).is_err(),
            "case{case} should reject"
        );
    }
}

#[test]
fn split_stale_and_dynamic_requests_leave_target_untouched() {
    let (mut doc, page, _, moving) = fixture(false, Color::WHITE);
    let spec = SplitSpec::prepare(&doc.scene, page, moving, &RenderInputs::empty(), None)
        .expect("prepare");
    let mut renderer = RasterRenderer::new(WIDTH, HEIGHT).expect("renderer");
    let mut target = surface();
    target.canvas().clear(skia_safe::Color::MAGENTA);
    let sentinel = pixels(&mut target);
    let viewport = Viewport {
        center: [0.0, 0.0],
        zoom: 1.0,
    };
    let playback = std::collections::HashMap::new();
    let inputs = RenderInputs {
        playback: Some(&playback),
        ..RenderInputs::empty()
    };
    assert_eq!(
        renderer
            .render_split_to_canvas(
                target.canvas(),
                WIDTH,
                HEIGHT,
                &doc.scene,
                &viewport,
                &inputs,
                &spec,
                SplitPhase::Middle
            )
            .err(),
        Some(SplitError::DynamicInputs)
    );
    doc.scene
        .set_transform(moving, Transform2D::translation(15.0, 10.0))
        .expect("move");
    assert_eq!(
        renderer
            .render_split_to_canvas(
                target.canvas(),
                WIDTH,
                HEIGHT,
                &doc.scene,
                &viewport,
                &RenderInputs::empty(),
                &spec,
                SplitPhase::Middle
            )
            .err(),
        Some(SplitError::StaleScene)
    );
    assert_eq!(pixels(&mut target), sentinel);
    assert!(matches!(
        SplitSpec::prepare(&doc.scene, page, page, &RenderInputs::empty(), None),
        Err(SplitError::InvalidRoots)
    ));
    assert!(matches!(
        SplitSpec::prepare(
            &doc.scene,
            page,
            NodeId::new(),
            &RenderInputs::empty(),
            None
        ),
        Err(SplitError::InvalidRoots)
    ));
}

#[test]
fn split_requires_decoded_assets_and_preserves_bitmap_pixels() {
    let (mut doc, page, _, moving) = fixture(false, Color::WHITE);
    let asset = fanta_doc::AssetId::new();
    doc.scene.get_mut(moving).expect("moving").data = NodeData::Bitmap(BitmapNode {
        asset,
        natural_size: [2, 2],
        local_size: [55.0, 46.0],
        crop: Some([0.0, 0.0, 0.75, 1.0]),
        fit: fanta_doc::ImageFitMode::Fill,
        tint: None,
    });
    assert!(
        matches!(SplitSpec::prepare(&doc.scene, page, moving, &RenderInputs::empty(), None), Err(SplitError::UnresolvedAsset(id)) if id == asset)
    );
    let mut resolver = crate::InMemoryAssetResolver::new();
    resolver.insert(
        asset,
        crate::DecodedImage::new(
            Arc::new(vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 128, 255, 255, 0, 200,
            ]),
            2,
            2,
        ),
    );
    assert_parity(
        &doc,
        page,
        moving,
        &Viewport {
            center: [0.0, 0.0],
            zoom: 1.0,
        },
        Some(Arc::new(resolver)),
    );
}

#[test]
fn split_rejects_static_sibling_masks_patterns_motion_and_unsupported_media() {
    for case in 0..7 {
        let (mut doc, page, _, moving) = fixture(false, Color::WHITE);
        let other = add(&mut doc, Some(page), rectangle(Color::WHITE), 75.0, 25.0);
        let node = doc.scene.get_mut(other).expect("sibling");
        match case {
            0 => node.is_mask = true,
            1 => node.blurs.push(Blur::background(2.0)),
            2 => node.data.as_vector_mut().expect("vector").fills[0] = Fill::Pattern {
                pattern: Box::new(fanta_doc::PatternFill { source_node_id: moving, tile_type: Default::default(), scaling_factor: 1.0, spacing: Default::default(), horizontal_alignment: Default::default() }),
                opacity: 1.0, blend: BlendMode::Normal,
            },
            3 => node.data = NodeData::Instance(InstanceNode { component: fanta_doc::ComponentId::new(), overrides: Vec::new(), prop_values: Default::default(), derived: Vec::new(), local_size: [20.0, 20.0] }),
            4 => node.data = serde_json::from_value(serde_json::json!({"type":"embed", "kind":"test", "local_size":[10,10], "payload":null})).expect("embed fixture"),
            5 => { node.bindings.insert(fanta_doc::BoundProp::Opacity, fanta_doc::VariableId::new()); },
            _ => node.data = NodeData::Video(fanta_doc::VideoNode { asset:fanta_doc::AssetId::new(), natural_size:[2,2], local_size:[10.0,10.0], time_range_us:[0,1], speed:1.0, muted:true, volume:1.0, poster_frame_us:None, poster:None, fit:fanta_doc::ImageFitMode::Fill }),
        }
        assert!(
            SplitSpec::prepare(&doc.scene, page, moving, &RenderInputs::empty(), None).is_err(),
            "case{case}"
        );
    }
    let (doc, page, _, moving) = fixture(false, Color::WHITE);
    let motion = fanta_doc::MotionEvaluation {
        clip: fanta_doc::AnimationClipId::new(),
        playhead_ms: 0,
        overrides: Default::default(),
    };
    let inputs = RenderInputs {
        motion: Some(&motion),
        ..RenderInputs::empty()
    };
    assert!(matches!(
        SplitSpec::prepare(&doc.scene, page, moving, &inputs, None),
        Err(SplitError::DynamicInputs)
    ));
}

#[test]
fn split_frozen_assets_do_not_follow_async_resolver_changes_between_phases() {
    struct Resolver(std::sync::Mutex<crate::DecodedImage>);
    impl AssetResolver for Resolver {
        fn resolve(&self, _: fanta_doc::AssetId) -> Option<crate::DecodedImage> {
            Some(self.0.lock().expect("resolver lock").clone())
        }
    }
    let (mut doc, page, _, moving) = fixture(false, Color::WHITE);
    let asset = fanta_doc::AssetId::new();
    doc.scene.get_mut(moving).expect("moving").data = NodeData::Bitmap(BitmapNode {
        asset,
        natural_size: [1, 1],
        local_size: [55.0, 46.0],
        crop: None,
        fit: fanta_doc::ImageFitMode::Fill,
        tint: None,
    });
    let resolver = Arc::new(Resolver(std::sync::Mutex::new(crate::DecodedImage::new(
        Arc::new(vec![240, 20, 30, 255]),
        1,
        1,
    ))));
    let inputs = RenderInputs::empty();
    let spec = SplitSpec::prepare(&doc.scene, page, moving, &inputs, Some(resolver.as_ref()))
        .expect("prepare");
    let viewport = Viewport {
        center: [0.0, 0.0],
        zoom: 1.0,
    };
    let mut renderer = RasterRenderer::new(WIDTH, HEIGHT).expect("renderer");
    renderer.set_asset_resolver(resolver.clone());
    let mut expected = surface();
    renderer.render_to_canvas(
        expected.canvas(),
        WIDTH,
        HEIGHT,
        &doc.scene,
        &viewport,
        Some(page),
        &inputs,
    );
    let render_phases = |renderer: &mut RasterRenderer| {
        let mut actual = surface();
        actual.canvas().clear(skia_safe::Color::TRANSPARENT);
        for phase in [SplitPhase::Below, SplitPhase::Middle, SplitPhase::Above] {
            let mut layer = surface();
            renderer
                .render_split_to_canvas(
                    layer.canvas(),
                    WIDTH,
                    HEIGHT,
                    &doc.scene,
                    &viewport,
                    &inputs,
                    &spec,
                    phase,
                )
                .expect("phase");
            actual
                .canvas()
                .draw_image(layer.image_snapshot(), (0, 0), None);
        }
        pixels(&mut actual)
    };
    let before_resolver_change = render_phases(&mut renderer);
    *resolver.0.lock().expect("resolver lock") =
        crate::DecodedImage::new(Arc::new(vec![20, 240, 30, 255]), 1, 1);
    let after_resolver_change = render_phases(&mut renderer);
    assert!(
        before_resolver_change == after_resolver_change,
        "prepared phases must remain pixel-identical after resolver mutation"
    );
    let normal_pixels = pixels(&mut expected);
    let maximum = after_resolver_change
        .iter()
        .zip(&normal_pixels)
        .map(|(actual, expected)| actual.abs_diff(*expected))
        .max()
        .expect("pixels");
    assert!(
        maximum <= 2,
        "composed phases versus normal max difference {maximum}"
    );
    let mut fresh = RasterRenderer::new(WIDTH, HEIGHT).expect("fresh");
    fresh.set_asset_resolver(resolver);
    let mut changed = surface();
    fresh.render_to_canvas(
        changed.canvas(),
        WIDTH,
        HEIGHT,
        &doc.scene,
        &viewport,
        Some(page),
        &inputs,
    );
    assert_ne!(
        pixels(&mut changed),
        pixels(&mut expected),
        "resolver mutation must be visible to an independent fresh normal render"
    );
}

#[test]
fn split_phase_pixels_contain_only_their_paint_atoms() {
    let mut doc = Doc::new();
    let page = add(
        &mut doc,
        None,
        NodeData::Group(GroupNode {
            background: Some(Fill::solid(Color::rgb(15, 25, 35))),
            ..Default::default()
        }),
        0.0,
        0.0,
    );
    add(
        &mut doc,
        Some(page),
        rectangle(Color::rgb(255, 0, 0)),
        -80.0,
        -70.0,
    );
    let moving = add(
        &mut doc,
        Some(page),
        rectangle(Color::rgb(0, 255, 0)),
        -20.0,
        -10.0,
    );
    add(
        &mut doc,
        Some(page),
        rectangle(Color::rgb(0, 0, 255)),
        40.0,
        45.0,
    );
    let inputs = RenderInputs::empty();
    let spec = SplitSpec::prepare(&doc.scene, page, moving, &inputs, None).expect("prepare");
    let mut renderer = RasterRenderer::new(WIDTH, HEIGHT).expect("renderer");
    for (phase, expected_color, point) in [
        (SplitPhase::Below, [255, 0, 0, 255], (20, 20)),
        (SplitPhase::Middle, [0, 255, 0, 255], (80, 75)),
        (SplitPhase::Above, [0, 0, 255, 255], (145, 140)),
    ] {
        let mut target = surface();
        let metrics = renderer
            .render_split_to_canvas(
                target.canvas(),
                WIDTH,
                HEIGHT,
                &doc.scene,
                &Viewport {
                    center: [0.0, 0.0],
                    zoom: 1.0,
                },
                &inputs,
                &spec,
                phase,
            )
            .expect("phase");
        assert_eq!(metrics.nodes_drawn, 1);
        let pixels = pixels(&mut target);
        assert_eq!(rgba_at(&pixels, WIDTH, point.0, point.1), expected_color);
        if phase != SplitPhase::Below {
            assert_eq!(rgba_at(&pixels, WIDTH, 0, 0), [0, 0, 0, 0]);
        } else {
            assert_eq!(rgba_at(&pixels, WIDTH, 0, 0), [15, 25, 35, 255]);
        }
    }
}

#[test]
fn split_invalid_viewport_and_replaced_scene_leave_canvas_untouched() {
    let (doc, page, _, moving) = fixture(false, Color::WHITE);
    let inputs = RenderInputs::empty();
    let spec = SplitSpec::prepare(&doc.scene, page, moving, &inputs, None).expect("prepare");
    let mut renderer = RasterRenderer::new(WIDTH, HEIGHT).expect("renderer");
    let mut target = surface();
    target.canvas().clear(skia_safe::Color::CYAN);
    let baseline = pixels(&mut target);
    for zoom in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::MIN_POSITIVE] {
        assert_eq!(
            renderer
                .render_split_to_canvas(
                    target.canvas(),
                    WIDTH,
                    HEIGHT,
                    &doc.scene,
                    &Viewport {
                        center: [0.0, 0.0],
                        zoom
                    },
                    &inputs,
                    &spec,
                    SplitPhase::Middle
                )
                .err(),
            Some(SplitError::InvalidViewport)
        );
        assert_eq!(pixels(&mut target), baseline);
    }
    assert_eq!(
        renderer
            .render_split_to_canvas(
                target.canvas(),
                WIDTH,
                HEIGHT,
                &doc.scene,
                &Viewport {
                    center: [f64::MAX, 0.0],
                    zoom: 1.0
                },
                &inputs,
                &spec,
                SplitPhase::Below
            )
            .expect_err("unrepresentable center"),
        SplitError::InvalidViewport
    );
    assert_eq!(pixels(&mut target), baseline);
    let replacement = doc.scene.clone();
    assert_eq!(
        renderer
            .render_split_to_canvas(
                target.canvas(),
                WIDTH,
                HEIGHT,
                &replacement,
                &Viewport {
                    center: [0.0, 0.0],
                    zoom: 1.0
                },
                &inputs,
                &spec,
                SplitPhase::Middle
            )
            .err(),
        Some(SplitError::StaleScene)
    );
    assert_eq!(pixels(&mut target), baseline);
}

#[test]
fn split_text_and_reflected_moving_subtree_match_complete_pixels() {
    let (mut doc, page, parent, _) = fixture(true, Color::WHITE);
    let moving = add(
        &mut doc,
        Some(parent),
        NodeData::Text(TextNode::new("Move me", 110.0, 40.0)),
        -25.0,
        -5.0,
    );
    doc.scene
        .set_transform(
            parent,
            Transform2D::scale_xy(-1.2, 0.8).then(&Transform2D::rotation(-0.17)),
        )
        .expect("reflect");
    assert_parity(
        &doc,
        page,
        moving,
        &Viewport {
            center: [0.0, 0.0],
            zoom: 1.3,
        },
        None,
    );
}

#[test]
fn split_rejects_nonfinite_path_effect_paint_and_text_scalars_hidden_by_bounds() {
    for case in 0..15 {
        let (mut doc, page, _, moving) = fixture(false, Color::WHITE);
        let node = doc.scene.get_mut(moving).expect("moving");
        match case {
            0 => {
                let NodeData::Vector(vector) = &mut node.data else {
                    panic!("vector");
                };
                vector.local_size = Some([55.0, 46.0]);
                vector.path.segments.push(fanta_doc::PathSegment::Cubic {
                    ctrl1: [f64::NAN, 0.0],
                    ctrl2: [2.0, 3.0],
                    to: [4.0, 5.0],
                });
            }
            1 => node.effects.push(Shadow {
                kind: ShadowKind::Drop,
                color: Color::BLACK,
                blur: f64::INFINITY,
                spread: 0.0,
                offset: [0.0, 0.0],
                show_behind_node: false,
            }),
            2 => node.blurs.push(Blur::layer(f64::NAN)),
            3..=5 => {
                let NodeData::Vector(vector) = &mut node.data else {
                    panic!("vector");
                };
                let mut stroke = fanta_doc::Stroke::solid(Color::BLACK, 2.0);
                match case {
                    3 => stroke.dash.push(f64::NAN),
                    4 => stroke.per_side = Some([1.0, 2.0, f64::INFINITY, 4.0]),
                    _ => stroke.miter_limit = f64::MAX,
                }
                vector.strokes.push(stroke);
            }
            6 => {
                let NodeData::Vector(vector) = &mut node.data else {
                    panic!("vector");
                };
                vector.corner_radii = Some([1.0, 2.0, f64::NAN, 4.0]);
            }
            7 => {
                let NodeData::Vector(vector) = &mut node.data else {
                    panic!("vector");
                };
                vector.fills.push(Fill::Gradient {
                    gradient: fanta_doc::Gradient::Linear {
                        start: [0.0, 0.0],
                        end: [1.0, 1.0],
                        stops: vec![fanta_doc::GradientStop {
                            position: f32::NAN,
                            color: Color::WHITE,
                        }],
                    },
                    blend: BlendMode::Normal,
                });
            }
            8 => {
                let NodeData::Vector(vector) = &mut node.data else {
                    panic!("vector");
                };
                vector.fills.push(Fill::Gradient {
                    gradient: fanta_doc::Gradient::Radial {
                        center: [0.5, 0.5],
                        radius: 0.5,
                        handles: Some([[0.5, 0.0], [f32::INFINITY, 0.5]]),
                        stops: Vec::new(),
                    },
                    blend: BlendMode::Normal,
                });
            }
            9 => {
                let mut text = TextNode::new("bad style", 80.0, 30.0);
                text.style_runs.push(TextStyleRun {
                    start: 0,
                    end: 3,
                    style: fanta_doc::TextStyle {
                        letter_spacing: f64::NAN,
                        ..Default::default()
                    },
                });
                node.data = NodeData::Text(text);
            }
            10 => {
                node.data = NodeData::Group(GroupNode {
                    local_size: Some([55.0, 46.0]),
                    corner_smoothing: f32::NAN,
                    ..Default::default()
                })
            }
            11 => node.transform = Transform2D::translation(f64::MAX, 0.0),
            12..=14 => {
                let NodeData::Vector(vector) = &mut node.data else {
                    panic!("vector");
                };
                vector.fills.push(Fill::Image {
                    asset: fanta_doc::AssetId::new(),
                    mode: fanta_doc::ImageFitMode::Fill,
                    opacity: if case == 12 { f32::NAN } else { 1.0 },
                    crop: if case == 13 {
                        Some(Box::new([0.0, f32::INFINITY, 1.0, 1.0]))
                    } else {
                        None
                    },
                    scale: None,
                    rotation: None,
                    blend: BlendMode::Normal,
                    adjust: fanta_doc::ImageAdjust {
                        contrast: if case == 14 { f32::NAN } else { 0.0 },
                        ..Default::default()
                    },
                });
            }
            _ => unreachable!(),
        }
        assert!(
            matches!(SplitSpec::prepare(&doc.scene, page, moving, &RenderInputs::empty(), None),
            Err(SplitError::InvalidGeometry(id)) if id == moving),
            "case {case}"
        );
    }
}

fn clipped_ancestor_fixture(
    reverse: bool,
    translucent: bool,
    reflected: bool,
) -> (Doc, NodeId, NodeId, NodeId) {
    let mut doc = Doc::new();
    let page = add(
        &mut doc,
        None,
        NodeData::Group(GroupNode {
            background: Some(Fill::solid(Color::rgba(
                15,
                22,
                35,
                if translucent { 70 } else { 255 },
            ))),
            ..Default::default()
        }),
        0.0,
        0.0,
    );
    add(
        &mut doc,
        Some(page),
        rectangle(Color::rgb(35, 60, 210)),
        -90.0,
        -70.0,
    );
    let frame = |width, height, color, align| {
        let mut stroke = fanta_doc::Stroke::solid(Color::rgba(240, 110, 30, 210), 3.5);
        stroke.align = align;
        NodeData::Group(GroupNode {
            clip_size: Some([width, height]),
            background: Some(Fill::solid(color)),
            corner_radii: Some([16.0, 8.0, 20.0, 11.0]),
            corner_smoothing: 0.35,
            strokes: [stroke].into_iter().collect(),
            auto_layout: Some(AutoLayout {
                reverse_z: reverse,
                ..Default::default()
            }),
            ..Default::default()
        })
    };
    let outer = add(
        &mut doc,
        Some(page),
        frame(
            144.0,
            119.0,
            Color::rgb(22, 30, 44),
            fanta_doc::StrokeAlign::Center,
        ),
        -72.25,
        -59.5,
    );
    if reflected {
        doc.scene
            .set_transform(
                outer,
                Transform2D::scale_xy(-1.0, 1.0)
                    .then(&Transform2D::rotation(0.09))
                    .then(&Transform2D::translation(70.25, -59.5)),
            )
            .expect("reflected ancestor");
    }
    add(
        &mut doc,
        Some(outer),
        rectangle(Color::rgba(30, 220, 60, 150)),
        -6.0,
        1.0,
    );
    let middle = add(
        &mut doc,
        Some(outer),
        frame(
            108.0,
            90.0,
            Color::rgba(45, 30, 75, 190),
            fanta_doc::StrokeAlign::Inside,
        ),
        12.75,
        9.25,
    );
    add(
        &mut doc,
        Some(middle),
        rectangle(Color::rgba(230, 70, 50, 150)),
        38.0,
        -8.0,
    );
    let inner = add(
        &mut doc,
        Some(middle),
        frame(
            85.0,
            66.0,
            Color::rgba(60, 80, 95, 210),
            fanta_doc::StrokeAlign::Outside,
        ),
        11.5,
        7.75,
    );
    doc.scene
        .get_mut(inner)
        .expect("inner")
        .effects
        .push(Shadow {
            kind: ShadowKind::Inner,
            color: Color::rgba(10, 0, 20, 150),
            blur: 5.0,
            spread: 1.5,
            offset: [2.0, -1.0],
            show_behind_node: false,
        });
    add(
        &mut doc,
        Some(inner),
        rectangle(Color::rgba(20, 110, 230, 170)),
        -4.0,
        -4.0,
    );
    let moving = add(
        &mut doc,
        Some(inner),
        NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            43.0,
            37.0,
            Color::rgba(245, 248, 250, if translucent { 160 } else { 255 }),
        )),
        0.0,
        0.0,
    );
    add(
        &mut doc,
        Some(inner),
        rectangle(Color::rgba(240, 40, 110, 100)),
        30.0,
        5.0,
    );
    let mut text = TextNode::new("Clip", 60.0, 24.0);
    text.style.size_px = 18.0;
    text.style.color = Color::rgba(245, 240, 10, 220);
    add(&mut doc, Some(middle), NodeData::Text(text), 5.0, 65.0);
    add(
        &mut doc,
        Some(outer),
        rectangle(Color::rgba(40, 210, 210, 190)),
        -13.0,
        82.0,
    );
    add(
        &mut doc,
        Some(page),
        rectangle(Color::rgba(20, 40, 170, 100)),
        40.0,
        20.0,
    );
    (doc, page, inner, moving)
}

#[test]
fn split_nested_ancestor_background_clip_and_foreground_match_all_pixels() {
    for reverse in [false, true] {
        for translucent in [false, true] {
            for reflected in [false, true] {
                let (mut doc, page, _, moving) =
                    clipped_ancestor_fixture(reverse, translucent, reflected);
                for [x, y] in [
                    [-12.0, -9.0],
                    [0.0, 0.0],
                    [40.0, 1.0],
                    [60.0, 43.0],
                    [1.0, 51.0],
                    [-25.0, 25.0],
                ] {
                    doc.scene
                        .set_transform(moving, Transform2D::translation(x, y))
                        .expect("move");
                    for (zoom, display_scale) in
                        [(0.13, 1.0), (0.73, 1.0), (1.0, 1.0), (1.0, 2.0), (2.0, 2.0)]
                    {
                        assert_parity_at_scale(
                            &doc,
                            page,
                            moving,
                            &Viewport {
                                center: [0.375, -1.125],
                                zoom,
                            },
                            None,
                            display_scale,
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn split_ancestor_phases_keep_background_border_and_child_clip_separate() {
    let mut doc = Doc::new();
    let page = add(
        &mut doc,
        None,
        NodeData::Group(GroupNode::default()),
        0.0,
        0.0,
    );
    let mut border = fanta_doc::Stroke::solid(Color::rgb(0, 0, 255), 4.0);
    border.align = fanta_doc::StrokeAlign::Outside;
    let parent = add(
        &mut doc,
        Some(page),
        NodeData::Group(GroupNode {
            clip_size: Some([60.0, 40.0]),
            background: Some(Fill::solid(Color::rgb(255, 0, 0))),
            strokes: [border].into_iter().collect(),
            ..Default::default()
        }),
        -30.0,
        -20.0,
    );
    let moving = add(
        &mut doc,
        Some(parent),
        rectangle(Color::rgb(0, 255, 0)),
        -10.0,
        4.0,
    );
    let inputs = RenderInputs::empty();
    let spec = SplitSpec::prepare(&doc.scene, page, moving, &inputs, None).expect("fixed frame");
    let mut renderer = RasterRenderer::new(WIDTH, HEIGHT).expect("renderer");
    for (phase, center, outside_border) in [
        (SplitPhase::Below, [255, 0, 0, 255], [0, 0, 0, 0]),
        (SplitPhase::Middle, [0, 255, 0, 255], [0, 0, 0, 0]),
        (SplitPhase::Above, [0, 0, 0, 0], [0, 0, 255, 255]),
    ] {
        let mut target = surface();
        renderer
            .paint_split_to_canvas(
                target.canvas(),
                WIDTH,
                HEIGHT,
                &doc.scene,
                &Viewport {
                    center: [0.0, 0.0],
                    zoom: 1.0,
                },
                &inputs,
                &spec,
                phase,
            )
            .expect("phase");
        let bytes = pixels(&mut target);
        assert_eq!(rgba_at(&bytes, WIDTH, 80, 75), center, "{phase:?} interior");
        assert_eq!(
            rgba_at(&bytes, WIDTH, 64, 70),
            outside_border,
            "{phase:?} own border must escape only own clip"
        );
        assert_eq!(
            rgba_at(&bytes, WIDTH, 58, 75),
            [0, 0, 0, 0],
            "{phase:?} child overflow must be clipped"
        );
    }
    assert_parity(
        &doc,
        page,
        moving,
        &Viewport {
            center: [0.0, 0.0],
            zoom: 1.0,
        },
        None,
    );
}

#[test]
fn split_ancestor_changes_invalidate_prepared_atoms_and_undo_preserves_pixels() {
    let (mut doc, page, parent, moving) = clipped_ancestor_fixture(false, false, false);
    let viewport = Viewport {
        center: [0.0, 0.0],
        zoom: 0.73,
    };
    let baseline = assert_parity(&doc, page, moving, &viewport, None);
    let inputs = RenderInputs::empty();
    let spec = SplitSpec::prepare(&doc.scene, page, moving, &inputs, None).expect("prepare");
    let old = doc.scene.get(parent).expect("parent").data.clone();
    let mut new = old.clone();
    new.as_group_mut().expect("group").corner_radius = Some(4.0);
    new.as_group_mut().expect("group").corner_radii = None;
    doc.apply(Operation::ReplaceData {
        id: parent,
        old: Box::new(old),
        new: Box::new(new),
    })
    .expect("change ancestor");
    let mut renderer = RasterRenderer::new(WIDTH, HEIGHT).expect("renderer");
    let mut target = surface();
    target.canvas().clear(skia_safe::Color::MAGENTA);
    let sentinel = pixels(&mut target);
    assert!(matches!(
        renderer.render_split_to_canvas(
            target.canvas(),
            WIDTH,
            HEIGHT,
            &doc.scene,
            &viewport,
            &inputs,
            &spec,
            SplitPhase::Above
        ),
        Err(SplitError::StaleScene)
    ));
    assert!(pixels(&mut target) == sentinel, "reject before drawing");
    assert_parity(&doc, page, moving, &viewport, None);
    assert!(doc.undo().expect("undo"));
    assert!(
        assert_parity(&doc, page, moving, &viewport, None) == baseline,
        "Undo restores all pixels"
    );
}

#[test]
fn split_ancestor_compositing_and_unbounded_paint_remain_ineligible() {
    for case in 0..10 {
        let (mut doc, page, parent, moving) = clipped_ancestor_fixture(false, false, false);
        let node = doc.scene.get_mut(parent).expect("ancestor");
        match case {
            0 => node.opacity = UnitInterval::new(0.5),
            1 => node.flags.insert(NodeFlags::ISOLATED_BLEND),
            2 => node.effects.push(Shadow {
                kind: ShadowKind::Drop,
                color: Color::BLACK,
                blur: 4.0,
                spread: 0.0,
                offset: [2.0, 1.0],
                show_behind_node: false,
            }),
            3 => node.blurs.push(Blur::layer(4.0)),
            4 => node.blurs.push(Blur::background(4.0)),
            5 => node.blend_mode = BlendMode::Multiply,
            6 => {
                node.meta = serde_json::json!({"clip_content": false});
            }
            7 => node.data.as_group_mut().expect("group").clip_size = None,
            8 => node.flags.insert(NodeFlags::HIDDEN),
            _ => {
                node.bindings
                    .insert(fanta_doc::BoundProp::Opacity, fanta_doc::VariableId::new());
            }
        }
        assert!(
            SplitSpec::prepare(&doc.scene, page, moving, &RenderInputs::empty(), None).is_err(),
            "case{case}"
        );
    }
}

#[test]
fn split_ancestor_gradient_and_decoded_image_backgrounds_match_full_render() {
    struct Resolver(fanta_doc::AssetId);
    impl AssetResolver for Resolver {
        fn resolve(&self, id: fanta_doc::AssetId) -> Option<crate::DecodedImage> {
            (id == self.0).then(|| {
                crate::DecodedImage::new(
                    Arc::new(vec![
                        240, 30, 20, 255, 20, 200, 80, 180, 30, 40, 210, 120, 180, 150, 30, 255,
                    ]),
                    2,
                    2,
                )
            })
        }
    }
    let (mut doc, page, parent, moving) = clipped_ancestor_fixture(false, true, false);
    let asset = fanta_doc::AssetId::new();
    let group = doc
        .scene
        .get_mut(parent)
        .expect("parent")
        .data
        .as_group_mut()
        .expect("group");
    group.background = Some(Fill::Gradient {
        gradient: fanta_doc::Gradient::Linear {
            start: [0.0, 0.0],
            end: [1.0, 1.0],
            stops: vec![
                fanta_doc::GradientStop {
                    position: 0.0,
                    color: Color::rgb(20, 60, 100),
                },
                fanta_doc::GradientStop {
                    position: 1.0,
                    color: Color::rgba(100, 20, 180, 150),
                },
            ],
        },
        blend: BlendMode::Normal,
    });
    group.background_fills.push(Fill::Image {
        asset,
        mode: fanta_doc::ImageFitMode::Fill,
        opacity: 0.7,
        crop: Some(Box::new([0.0, 0.0, 0.75, 1.0])),
        scale: None,
        rotation: None,
        blend: BlendMode::Normal,
        adjust: Default::default(),
    });
    assert!(
        matches!(SplitSpec::prepare(&doc.scene, page, moving, &RenderInputs::empty(), None), Err(SplitError::UnresolvedAsset(id)) if id == asset)
    );
    let resolver: Arc<dyn AssetResolver> = Arc::new(Resolver(asset));
    for zoom in [0.73, 1.0, 2.0] {
        assert_parity_at_scale(
            &doc,
            page,
            moving,
            &Viewport {
                center: [0.0, 0.0],
                zoom,
            },
            Some(resolver.clone()),
            2.0,
        );
    }
}

#[test]
fn split_ancestor_clip_without_paint_keeps_empty_above_and_rejects_missing_instances() {
    let mut doc = Doc::new();
    let page = add(
        &mut doc,
        None,
        NodeData::Group(GroupNode::default()),
        0.0,
        0.0,
    );
    let parent = add(
        &mut doc,
        Some(page),
        NodeData::Group(GroupNode {
            clip_size: Some([50.0, 40.0]),
            corner_radius: Some(8.0),
            ..Default::default()
        }),
        -25.0,
        -20.0,
    );
    let moving = add(&mut doc, Some(parent), rectangle(Color::WHITE), -4.0, -4.0);
    let inputs = RenderInputs::empty();
    let spec =
        SplitSpec::prepare(&doc.scene, page, moving, &inputs, None).expect("clip-only ancestor");
    let mut target = surface();
    let mut renderer = RasterRenderer::new(WIDTH, HEIGHT).expect("renderer");
    let metrics = renderer
        .paint_split_to_canvas(
            target.canvas(),
            WIDTH,
            HEIGHT,
            &doc.scene,
            &Viewport {
                center: [0.0, 0.0],
                zoom: 1.0,
            },
            &inputs,
            &spec,
            SplitPhase::Above,
        )
        .expect("Above");
    assert_eq!(metrics.nodes_drawn, 0);
    assert!(pixels(&mut target).into_iter().all(|channel| channel == 0));
    let mut recorder = skia_safe::PictureRecorder::new();
    let recording =
        recorder.begin_recording(skia_safe::Rect::from_wh(WIDTH as f32, HEIGHT as f32), None);
    renderer
        .paint_split_to_canvas(
            recording,
            WIDTH,
            HEIGHT,
            &doc.scene,
            &Viewport {
                center: [0.0, 0.0],
                zoom: 1.0,
            },
            &inputs,
            &spec,
            SplitPhase::Above,
        )
        .expect("record empty Above");
    let picture = recorder
        .finish_recording_as_picture(None)
        .expect("empty Above picture");
    target.canvas().clear(skia_safe::Color::MAGENTA);
    let before = pixels(&mut target);
    target.canvas().draw_picture(picture, None, None);
    assert_eq!(
        pixels(&mut target),
        before,
        "empty Above replay must not clear earlier artwork"
    );
    doc.scene.get_mut(moving).expect("moving").data = NodeData::Instance(InstanceNode {
        component: fanta_doc::ComponentId::new(),
        overrides: Vec::new(),
        prop_values: Default::default(),
        derived: Vec::new(),
        local_size: [20.0, 20.0],
    });
    assert!(
        matches!(SplitSpec::prepare(&doc.scene, page, moving, &inputs, None), Err(SplitError::MissingInstance { instance, .. }) if instance == moving)
    );
}

#[test]
fn split_ancestor_raster_phases_refuse_before_modifying_the_target() {
    let (doc, page, _, moving) = clipped_ancestor_fixture(false, false, false);
    let inputs = RenderInputs::empty();
    let spec = SplitSpec::prepare(&doc.scene, page, moving, &inputs, None).expect("prepare");
    assert!(spec.requires_ordered_paint());
    let mut renderer = RasterRenderer::new(WIDTH, HEIGHT).expect("renderer");
    let mut target = surface();
    target.canvas().clear(skia_safe::Color::MAGENTA);
    let before = pixels(&mut target);
    for phase in [SplitPhase::Below, SplitPhase::Middle, SplitPhase::Above] {
        assert!(matches!(
            renderer.render_split_to_canvas(
                target.canvas(),
                WIDTH,
                HEIGHT,
                &doc.scene,
                &Viewport {
                    center: [0.375, -1.125],
                    zoom: 0.13
                },
                &inputs,
                &spec,
                phase,
            ),
            Err(SplitError::RequiresOrderedPaint)
        ));
        assert_eq!(pixels(&mut target), before);
    }
}

#[test]
fn split_ordered_phases_preserve_incoming_canvas_state_and_reject_stale_scene() {
    let (mut doc, page, _, moving) = clipped_ancestor_fixture(true, true, true);
    let inputs = RenderInputs::empty();
    let spec = SplitSpec::prepare(&doc.scene, page, moving, &inputs, None).expect("prepare");
    let viewport = Viewport {
        center: [0.375, -1.125],
        zoom: 0.13,
    };
    let mut renderer = RasterRenderer::new(WIDTH, HEIGHT).expect("renderer");
    let mut expected = surface();
    let mut actual = surface();
    for target in [&mut expected, &mut actual] {
        target.canvas().clear(skia_safe::Color::MAGENTA);
        target.canvas().translate((2.5, 1.5));
        target.canvas().clip_rect(
            skia_safe::Rect::from_xywh(4.0, 3.0, 170.0, 148.0),
            None,
            true,
        );
    }
    renderer.render_to_canvas(
        expected.canvas(),
        WIDTH,
        HEIGHT,
        &doc.scene,
        &viewport,
        Some(page),
        &inputs,
    );
    let matrix = actual.canvas().local_to_device();
    let clip = actual.canvas().device_clip_bounds();
    let saves = actual.canvas().save_count();
    for phase in [SplitPhase::Below, SplitPhase::Middle, SplitPhase::Above] {
        renderer
            .paint_split_to_canvas(
                actual.canvas(),
                WIDTH,
                HEIGHT,
                &doc.scene,
                &viewport,
                &inputs,
                &spec,
                phase,
            )
            .expect("paint phase");
        assert_eq!(actual.canvas().local_to_device(), matrix);
        assert_eq!(actual.canvas().device_clip_bounds(), clip);
        assert_eq!(actual.canvas().save_count(), saves);
    }
    assert_eq!(
        pixels(&mut actual),
        pixels(&mut expected),
        "ordered paint keeps original arithmetic and external clip"
    );
    doc.scene
        .set_transform(moving, Transform2D::translation(13.0, 8.0))
        .expect("change scene");
    let before = pixels(&mut actual);
    assert!(matches!(
        renderer.paint_split_to_canvas(
            actual.canvas(),
            WIDTH,
            HEIGHT,
            &doc.scene,
            &viewport,
            &inputs,
            &spec,
            SplitPhase::Middle
        ),
        Err(SplitError::StaleScene)
    ));
    assert_eq!(pixels(&mut actual), before);
    assert_eq!(actual.canvas().local_to_device(), matrix);
    assert_eq!(actual.canvas().device_clip_bounds(), clip);
    assert_eq!(actual.canvas().save_count(), saves);
}

#[test]
fn split_plain_ancestor_raster_phases_keep_low_zoom_parity() {
    for reverse in [false, true] {
        for translucent in [false, true] {
            for reflected in [false, true] {
                let (mut doc, page, _, moving) =
                    clipped_ancestor_fixture(reverse, translucent, reflected);
                let ancestors: Vec<_> = doc
                    .scene
                    .ancestors_of(moving)
                    .map(|node| node.id)
                    .filter(|id| *id != page)
                    .collect();
                for id in ancestors {
                    let node = doc.scene.get_mut(id).expect("ancestor");
                    node.effects.clear();
                    node.data = NodeData::Group(GroupNode {
                        auto_layout: Some(AutoLayout {
                            reverse_z: reverse,
                            ..Default::default()
                        }),
                        ..Default::default()
                    });
                }
                let inputs = RenderInputs::empty();
                let spec = SplitSpec::prepare(&doc.scene, page, moving, &inputs, None)
                    .expect("plain ancestors");
                assert!(!spec.requires_ordered_paint());
                for [x, y] in [
                    [-12.0, -9.0],
                    [0.0, 0.0],
                    [40.0, 1.0],
                    [60.0, 43.0],
                    [1.0, 51.0],
                    [-25.0, 25.0],
                ] {
                    doc.scene
                        .set_transform(moving, Transform2D::translation(x, y))
                        .expect("move");
                    assert_parity(
                        &doc,
                        page,
                        moving,
                        &Viewport {
                            center: [0.375, -1.125],
                            zoom: 0.13,
                        },
                        None,
                    );
                }
            }
        }
    }
}

fn whole_instance(component: fanta_doc::ComponentId, size: [f64; 2]) -> fanta_doc::InstanceNode {
    fanta_doc::InstanceNode {
        component,
        overrides: Vec::new(),
        prop_values: BTreeMap::new(),
        derived: Vec::new(),
        local_size: size,
    }
}

fn instance_page() -> (Doc, NodeId) {
    let mut doc = Doc::new();
    let page = add(
        &mut doc,
        None,
        NodeData::Group(GroupNode {
            background: Some(Fill::solid(Color::BLACK)),
            ..Default::default()
        }),
        0.0,
        0.0,
    );
    (doc, page)
}

fn solid_master(doc: &mut Doc, color: Color) -> (fanta_doc::ComponentId, NodeId) {
    let root = add(
        doc,
        None,
        NodeData::Vector(VectorNode::rect_solid(0.0, 0.0, 20.0, 20.0, color)),
        10_000.0,
        5_000.0,
    );
    let component = fanta_doc::ComponentId::new();
    doc.components.defs.insert(
        component,
        fanta_doc::ComponentDef::new(component, root, "Solid"),
    );
    (component, root)
}

#[test]
fn split_whole_instances_preserve_derived_geometry_and_sparse_paints() {
    use fanta_doc::{
        BoundProp, ComponentDef, ComponentId, DerivedOverride, LayoutMode, Override, OverrideValue,
    };
    use smallvec::smallvec;
    for reverse in [false, true] {
        for reflected in [false, true] {
            let (mut doc, page) = instance_page();
            let frame = add(
                &mut doc,
                Some(page),
                NodeData::Group(GroupNode {
                    clip_size: Some([160.0, 100.0]),
                    auto_layout: Some(AutoLayout {
                        reverse_z: reverse,
                        ..Default::default()
                    }),
                    background: Some(Fill::solid(Color::rgb(25, 30, 35))),
                    ..Default::default()
                }),
                -80.0,
                -50.0,
            );
            if reflected {
                doc.scene.get_mut(frame).expect("frame").transform =
                    Transform2D::scale_xy(-1.0, 1.0).then(&Transform2D::translation(80.0, -50.0));
            }
            let root = add(
                &mut doc,
                None,
                NodeData::Group(GroupNode {
                    clip_size: Some([40.0, 30.0]),
                    auto_layout: Some(AutoLayout {
                        mode: LayoutMode::Horizontal,
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                10_000.0,
                5_000.0,
            );
            let child = add(
                &mut doc,
                Some(root),
                NodeData::Vector(VectorNode::rect_solid(
                    0.0,
                    0.0,
                    30.0,
                    22.0,
                    Color::rgb(255, 0, 0),
                )),
                0.0,
                0.0,
            );
            let component = ComponentId::new();
            doc.components.defs.insert(
                component,
                ComponentDef::new(component, root, "Derived placement"),
            );
            add(
                &mut doc,
                Some(frame),
                NodeData::Instance(whole_instance(component, [40.0, 30.0])),
                12.0,
                24.0,
            );
            let mut instance = whole_instance(component, [40.0, 30.0]);
            instance.derived.push(DerivedOverride {
                path: smallvec![child],
                transform: Some(Transform2D::translation(20.0, 10.0)),
                size: Some([12.0, 9.0]),
                fills: Some(smallvec![Fill::solid(Color::rgb(0, 255, 0))]),
                path_data: None,
                stroke_path: None,
                stroke_weight: None,
                text: None,
            });
            instance.overrides.push(Override {
                target_path: smallvec![child],
                target_prop: BoundProp::FillColor { index: 0 },
                value: OverrideValue::Fills {
                    fills: smallvec![Fill::solid(Color::rgb(0, 0, 255))],
                },
            });
            let moving = add(
                &mut doc,
                Some(frame),
                NodeData::Instance(instance),
                60.0,
                24.0,
            );
            add(
                &mut doc,
                Some(frame),
                NodeData::Instance(whole_instance(component, [40.0, 30.0])),
                112.0,
                24.0,
            );
            let inputs = RenderInputs::for_doc(&doc);
            for zoom in [0.13, 0.73, 1.0, 2.0] {
                for scale in [1.0, 2.0] {
                    assert_parity_with_inputs(
                        &doc,
                        page,
                        moving,
                        &Viewport {
                            center: [0.375, -0.625],
                            zoom,
                        },
                        None,
                        scale,
                        &inputs,
                    );
                }
            }
            if !reflected {
                let pixels = assert_parity_with_inputs(
                    &doc,
                    page,
                    moving,
                    &Viewport {
                        center: [0.0, 0.0],
                        zoom: 1.0,
                    },
                    None,
                    1.0,
                    &inputs,
                );
                assert_eq!(
                    rgba_at(&pixels, WIDTH, 102, 69),
                    [0, 0, 255, 255],
                    "explicit paint wins over imported derived fill at the imported offset"
                );
                assert_eq!(
                    rgba_at(&pixels, WIDTH, 80, 57),
                    [25, 30, 35, 255],
                    "derived geometry must not reflow to the left edge"
                );
            }
        }
    }
}

struct InstanceTheme {
    collection: fanta_doc::VariableCollectionId,
    light: fanta_doc::ModeId,
    dark: fanta_doc::ModeId,
    color: fanta_doc::VariableId,
    placed_color: fanta_doc::VariableId,
    opacity: fanta_doc::VariableId,
    variant: fanta_doc::VariableId,
}

fn add_instance_theme(doc: &mut Doc) -> InstanceTheme {
    use fanta_doc::{
        Mode, ModeId, VarValue, Variable, VariableCollection, VariableCollectionId, VariableId,
        VariableType,
    };
    let theme = InstanceTheme {
        collection: VariableCollectionId::new(),
        light: ModeId::new(),
        dark: ModeId::new(),
        color: VariableId::new(),
        placed_color: VariableId::new(),
        opacity: VariableId::new(),
        variant: VariableId::new(),
    };
    doc.variables.collections.insert(
        theme.collection,
        VariableCollection {
            id: theme.collection,
            name: "Theme".into(),
            modes: vec![
                Mode {
                    id: theme.light,
                    name: "Light".into(),
                },
                Mode {
                    id: theme.dark,
                    name: "Dark".into(),
                },
            ],
            default_mode: theme.light,
            variable_order: vec![
                theme.color,
                theme.placed_color,
                theme.opacity,
                theme.variant,
            ],
        },
    );
    for (id, name, ty, light, dark) in [
        (
            theme.color,
            "Default color",
            VariableType::Color,
            VarValue::Color {
                value: Color::WHITE,
            },
            VarValue::Color {
                value: Color::rgb(0, 0, 255),
            },
        ),
        (
            theme.placed_color,
            "Placed color",
            VariableType::Color,
            VarValue::Color {
                value: Color::rgb(255, 0, 0),
            },
            VarValue::Color {
                value: Color::rgb(255, 255, 0),
            },
        ),
        (
            theme.opacity,
            "Opacity",
            VariableType::Float,
            VarValue::Float { value: 0.5 },
            VarValue::Float { value: 1.0 },
        ),
        (
            theme.variant,
            "Variant",
            VariableType::String,
            VarValue::String {
                value: "Red".into(),
            },
            VarValue::String {
                value: "Blue".into(),
            },
        ),
    ] {
        doc.variables.variables.insert(
            id,
            Variable {
                id,
                collection: theme.collection,
                name: name.into(),
                ty,
                values_by_mode: BTreeMap::from([(theme.light, light), (theme.dark, dark)]),
                scopes: Vec::new(),
            },
        );
    }
    theme
}

#[test]
fn split_whole_instances_resolve_defaults_placed_aliases_and_nested_real_mode_anchor() {
    use fanta_doc::{
        BoundProp, ComponentDef, ComponentId, ComponentPropDef, ComponentPropId, ComponentPropKind,
        PropBindingTarget, VarValue,
    };
    use smallvec::smallvec;
    let (mut doc, page) = instance_page();
    let theme = add_instance_theme(&mut doc);
    let (leaf, root) = solid_master(&mut doc, Color::rgb(255, 0, 255));
    doc.scene
        .get_mut(root)
        .expect("master")
        .bindings
        .insert(BoundProp::Opacity, theme.opacity);
    let property = ComponentPropId::new();
    doc.components
        .defs
        .get_mut(&leaf)
        .expect("definition")
        .props
        .push(ComponentPropDef {
            id: property,
            name: "Surface".into(),
            kind: ComponentPropKind::Color,
            formatter: Default::default(),
            default: VarValue::Alias {
                variable: theme.color,
            },
            bindings: vec![PropBindingTarget {
                path: smallvec![],
                prop: BoundProp::FillColor { index: 0 },
            }],
        });
    let outer_root = add(
        &mut doc,
        None,
        NodeData::Group(GroupNode {
            clip_size: Some([20.0, 20.0]),
            explicit_modes: BTreeMap::from([(theme.collection, theme.light)]),
            ..Default::default()
        }),
        10_000.0,
        0.0,
    );
    add(
        &mut doc,
        Some(outer_root),
        NodeData::Instance(whole_instance(leaf, [20.0, 20.0])),
        0.0,
        0.0,
    );
    let outer = ComponentId::new();
    doc.components
        .defs
        .insert(outer, ComponentDef::new(outer, outer_root, "Nested"));
    let pinned = add(
        &mut doc,
        Some(page),
        NodeData::Group(GroupNode {
            explicit_modes: BTreeMap::from([(theme.collection, theme.dark)]),
            ..Default::default()
        }),
        -70.0,
        -10.0,
    );
    let moving = add(
        &mut doc,
        Some(pinned),
        NodeData::Instance(whole_instance(outer, [20.0, 20.0])),
        0.0,
        0.0,
    );
    add(
        &mut doc,
        Some(page),
        NodeData::Instance(whole_instance(leaf, [20.0, 20.0])),
        -10.0,
        -10.0,
    );
    let mut placed = whole_instance(leaf, [20.0, 20.0]);
    placed.prop_values.insert(
        property,
        VarValue::Alias {
            variable: theme.placed_color,
        },
    );
    add(
        &mut doc,
        Some(page),
        NodeData::Instance(placed),
        50.0,
        -10.0,
    );
    for dark in [false, true] {
        doc.active_modes = if dark {
            BTreeMap::from([(theme.collection, theme.dark)])
        } else {
            BTreeMap::new()
        };
        let inputs = RenderInputs::for_doc(&doc);
        let pixels = assert_parity_with_inputs(
            &doc,
            page,
            moving,
            &Viewport {
                center: [0.0, 0.0],
                zoom: 1.0,
            },
            None,
            1.0,
            &inputs,
        );
        assert_eq!(
            rgba_at(&pixels, WIDTH, 36, 80),
            [0, 0, 255, 255],
            "nested expansion inherits the real placed pin, not the master frame pin or fresh clone ID"
        );
        let default = rgba_at(&pixels, WIDTH, 96, 80);
        let placed = rgba_at(&pixels, WIDTH, 156, 80);
        if dark {
            assert_eq!(default, [0, 0, 255, 255]);
            assert_eq!(placed, [255, 255, 0, 255]);
        } else {
            assert!(
                default[0].abs_diff(128) <= 1
                    && default[1] == default[0]
                    && default[2] == default[0]
                    && default[3] == 255,
                "default color alias and opacity binding: {default:?}"
            );
            assert!(
                placed[0].abs_diff(128) <= 1 && placed[1..] == [0, 0, 255],
                "placed alias replaces the default: {placed:?}"
            );
        }
    }
}

#[test]
fn split_whole_instances_resolve_variant_aliases_and_nested_sparse_swaps() {
    use fanta_doc::{
        BoundProp, ComponentDef, ComponentId, ComponentPropDef, ComponentPropId, ComponentPropKind,
        ComponentSet, ComponentSetMembership, Override, OverrideValue, VarValue, VariantAxis,
    };
    use smallvec::smallvec;
    let (mut doc, page) = instance_page();
    let theme = add_instance_theme(&mut doc);
    let (red, _) = solid_master(&mut doc, Color::rgb(255, 0, 0));
    let (blue, _) = solid_master(&mut doc, Color::rgb(0, 0, 255));
    let set = ComponentId::new();
    let property = ComponentPropId::new();
    for (component, label) in [(red, "Red"), (blue, "Blue")] {
        let def = doc.components.defs.get_mut(&component).expect("variant");
        def.variant_of = Some(ComponentSetMembership {
            set,
            axis_values: BTreeMap::from([("Color".into(), label.into())]),
        });
        def.props.push(ComponentPropDef {
            id: property,
            name: "Color".into(),
            kind: ComponentPropKind::Variant {
                axis: "Color".into(),
            },
            formatter: Default::default(),
            default: VarValue::Alias {
                variable: theme.variant,
            },
            bindings: Vec::new(),
        });
    }
    doc.components.sets.insert(
        set,
        ComponentSet {
            id: set,
            name: "Color".into(),
            axes: vec![VariantAxis {
                name: "Color".into(),
                values: vec!["Red".into(), "Blue".into()],
            }],
            members: vec![red, blue],
            default_variant: red,
            root: None,
        },
    );
    let outer_root = add(
        &mut doc,
        None,
        NodeData::Group(GroupNode {
            clip_size: Some([20.0, 20.0]),
            ..Default::default()
        }),
        10_000.0,
        0.0,
    );
    let mut nested_instance = whole_instance(red, [20.0, 20.0]);
    nested_instance.prop_values.insert(
        property,
        VarValue::Alias {
            variable: theme.variant,
        },
    );
    let nested = add(
        &mut doc,
        Some(outer_root),
        NodeData::Instance(nested_instance),
        0.0,
        0.0,
    );
    let outer = ComponentId::new();
    doc.components
        .defs
        .insert(outer, ComponentDef::new(outer, outer_root, "Swap owner"));
    let mut swapped = whole_instance(outer, [20.0, 20.0]);
    swapped.overrides.push(Override {
        target_path: smallvec![nested],
        target_prop: BoundProp::Visible,
        value: OverrideValue::SwapInstance { component: set },
    });
    let moving = add(
        &mut doc,
        Some(page),
        NodeData::Instance(swapped),
        -40.0,
        -10.0,
    );
    let mut placed = whole_instance(set, [20.0, 20.0]);
    placed.prop_values.insert(
        property,
        VarValue::String {
            value: "Red".into(),
        },
    );
    add(
        &mut doc,
        Some(page),
        NodeData::Instance(placed),
        20.0,
        -10.0,
    );
    for dark in [false, true] {
        doc.active_modes = if dark {
            BTreeMap::from([(theme.collection, theme.dark)])
        } else {
            BTreeMap::new()
        };
        let pixels = assert_parity_with_inputs(
            &doc,
            page,
            moving,
            &Viewport {
                center: [0.0, 0.0],
                zoom: 1.0,
            },
            None,
            1.0,
            &RenderInputs::for_doc(&doc),
        );
        assert_eq!(
            rgba_at(&pixels, WIDTH, 66, 80),
            if dark {
                [0, 0, 255, 255]
            } else {
                [255, 0, 0, 255]
            }
        );
        assert_eq!(
            rgba_at(&pixels, WIDTH, 126, 80),
            [255, 0, 0, 255],
            "placed variant value beats the aliased default"
        );
    }
}

#[test]
fn split_whole_instances_reject_changed_input_identity_before_touching_the_target() {
    let (mut doc, page) = instance_page();
    let theme = add_instance_theme(&mut doc);
    let (component, _) = solid_master(&mut doc, Color::WHITE);
    let moving = add(
        &mut doc,
        Some(page),
        NodeData::Instance(whole_instance(component, [20.0, 20.0])),
        -10.0,
        -10.0,
    );
    let original = RenderInputs::for_doc(&doc);
    let spec =
        SplitSpec::prepare(&doc.scene, page, moving, &original, None).expect("instance plan");
    let mut changed_components = doc.components.clone();
    changed_components
        .defs
        .get_mut(&component)
        .expect("definition")
        .name = "Changed without generation".into();
    let mut changed_variables = doc.variables.clone();
    changed_variables
        .variables
        .get_mut(&theme.opacity)
        .expect("variable")
        .values_by_mode
        .insert(theme.light, fanta_doc::VarValue::Float { value: 0.75 });
    let changed_modes = BTreeMap::from([(theme.collection, theme.dark)]);
    let mut renderer = RasterRenderer::new(WIDTH, HEIGHT).expect("renderer");
    let mut target = surface();
    target.canvas().clear(skia_safe::Color::MAGENTA);
    let untouched = pixels(&mut target);
    for inputs in [
        RenderInputs {
            components: &changed_components,
            ..RenderInputs::for_doc(&doc)
        },
        RenderInputs {
            variables: &changed_variables,
            ..RenderInputs::for_doc(&doc)
        },
        RenderInputs {
            active_modes: &changed_modes,
            ..RenderInputs::for_doc(&doc)
        },
        RenderInputs {
            mode_generation: original.mode_generation + 1,
            ..RenderInputs::for_doc(&doc)
        },
        RenderInputs {
            dark_ui: !original.dark_ui,
            ..RenderInputs::for_doc(&doc)
        },
    ] {
        assert!(
            renderer
                .paint_split_to_canvas(
                    target.canvas(),
                    WIDTH,
                    HEIGHT,
                    &doc.scene,
                    &Viewport {
                        center: [0.0, 0.0],
                        zoom: 1.0
                    },
                    &inputs,
                    &spec,
                    SplitPhase::Below
                )
                .is_err()
        );
        assert_eq!(
            pixels(&mut target),
            untouched,
            "failed validation must not clear or paint"
        );
    }
    for phase in [SplitPhase::Below, SplitPhase::Middle, SplitPhase::Above] {
        renderer
            .paint_split_to_canvas(
                target.canvas(),
                WIDTH,
                HEIGHT,
                &doc.scene,
                &Viewport {
                    center: [0.0, 0.0],
                    zoom: 1.0,
                },
                &original,
                &spec,
                phase,
            )
            .expect("original inputs remain valid");
    }
    assert_eq!(
        rgba_at(&pixels(&mut target), WIDTH, 96, 80),
        [255, 255, 255, 255]
    );
    assert!(
        renderer
            .render_split_to_canvas(
                target.canvas(),
                WIDTH,
                HEIGHT,
                &doc.scene,
                &Viewport {
                    center: [0.0, 0.0],
                    zoom: 1.0
                },
                &original,
                &spec,
                SplitPhase::Middle
            )
            .is_err(),
        "instance plans must use ordered commands"
    );
}

#[test]
fn split_whole_instances_reject_unsupported_resolved_contents_and_cycles() {
    use fanta_doc::{BoundProp, ComponentDef, ComponentId, Override, OverrideValue, VariableId};
    use smallvec::smallvec;
    for case in 0..6 {
        let (mut doc, page) = instance_page();
        let (leaf, root) = solid_master(&mut doc, Color::WHITE);
        let outer_root = add(
            &mut doc,
            None,
            NodeData::Group(GroupNode {
                clip_size: Some([20.0, 20.0]),
                ..Default::default()
            }),
            10_000.0,
            0.0,
        );
        let nested = add(
            &mut doc,
            Some(outer_root),
            NodeData::Instance(whole_instance(leaf, [20.0, 20.0])),
            0.0,
            0.0,
        );
        let outer = ComponentId::new();
        doc.components
            .defs
            .insert(outer, ComponentDef::new(outer, outer_root, "Outer"));
        let mut placed = whole_instance(outer, [20.0, 20.0]);
        let expected = match case {
            0 => {
                doc.scene.get_mut(root).expect("leaf").is_mask = true;
                "masks"
            }
            1 => {
                doc.scene.get_mut(root).expect("leaf").blend_mode = BlendMode::Screen;
                "backdrop"
            }
            2 => {
                doc.scene
                    .get_mut(root)
                    .expect("leaf")
                    .bindings
                    .insert(BoundProp::Opacity, VariableId::new());
                "variable-bound"
            }
            3 => {
                placed.overrides.push(Override {
                    target_path: smallvec![nested],
                    target_prop: BoundProp::Visible,
                    value: OverrideValue::SwapInstance {
                        component: ComponentId::new(),
                    },
                });
                "missing component"
            }
            4 => {
                doc.scene.get_mut(root).expect("leaf").data =
                    NodeData::Instance(whole_instance(outer, [20.0, 20.0]));
                "recursive instance"
            }
            5 => {
                placed.overrides.push(Override {
                    target_path: smallvec![nested],
                    target_prop: BoundProp::Visible,
                    value: OverrideValue::Field {
                        value: serde_json::json!({"local_size": [1.0e300, 20.0]}),
                    },
                });
                "finite geometry"
            }
            _ => unreachable!(),
        };
        let moving = add(
            &mut doc,
            Some(page),
            NodeData::Instance(placed),
            -10.0,
            -10.0,
        );
        let before = serde_json::to_value(&doc).expect("before");
        let error = match SplitSpec::prepare(
            &doc.scene,
            page,
            moving,
            &RenderInputs::for_doc(&doc),
            None,
        ) {
            Ok(_) => panic!("unsafe instance case {case} was accepted"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains(expected), "case {case}: {error}");
        assert!(
            error.contains("definition path"),
            "resolved failures should identify the instance path: {error}"
        );
        assert_eq!(serde_json::to_value(&doc).expect("after"), before);
    }
}

#[test]
fn split_whole_instances_bound_recursive_work_but_allow_repeated_masters() {
    use fanta_doc::{ComponentDef, ComponentId};
    let (mut doc, page) = instance_page();
    let (leaf, _) = solid_master(&mut doc, Color::WHITE);
    let moving = add(
        &mut doc,
        Some(page),
        NodeData::Instance(whole_instance(leaf, [20.0, 20.0])),
        -20.0,
        -10.0,
    );
    add(
        &mut doc,
        Some(page),
        NodeData::Instance(whole_instance(leaf, [20.0, 20.0])),
        10.0,
        -10.0,
    );
    assert_parity_with_inputs(
        &doc,
        page,
        moving,
        &Viewport {
            center: [0.0, 0.0],
            zoom: 1.0,
        },
        None,
        1.0,
        &RenderInputs::for_doc(&doc),
    );
    let mut next = leaf;
    for _ in 0..65 {
        let root = add(
            &mut doc,
            None,
            NodeData::Instance(whole_instance(next, [20.0, 20.0])),
            10_000.0,
            0.0,
        );
        let component = ComponentId::new();
        doc.components
            .defs
            .insert(component, ComponentDef::new(component, root, "Chain"));
        next = component;
    }
    doc.scene.get_mut(moving).expect("moving").data =
        NodeData::Instance(whole_instance(next, [20.0, 20.0]));
    let error =
        match SplitSpec::prepare(&doc.scene, page, moving, &RenderInputs::for_doc(&doc), None) {
            Ok(_) => panic!("unbounded expansion was accepted"),
            Err(error) => error.to_string(),
        };
    assert!(error.contains("node/depth budget"), "{error}");
}

#[test]
fn split_whole_instances_freeze_nested_bitmap_assets_between_phases() {
    use fanta_doc::{AssetId, ComponentDef, ComponentId};
    struct Resolver(std::sync::Mutex<crate::DecodedImage>);
    impl AssetResolver for Resolver {
        fn resolve(&self, _: AssetId) -> Option<crate::DecodedImage> {
            Some(self.0.lock().expect("resolver").clone())
        }
    }
    let (mut doc, page) = instance_page();
    let root = add(
        &mut doc,
        None,
        NodeData::Bitmap(BitmapNode {
            asset: AssetId::new(),
            natural_size: [1, 1],
            local_size: [20.0, 20.0],
            crop: None,
            fit: fanta_doc::ImageFitMode::Fill,
            tint: None,
        }),
        10_000.0,
        0.0,
    );
    let leaf = ComponentId::new();
    doc.components
        .defs
        .insert(leaf, ComponentDef::new(leaf, root, "Bitmap"));
    let outer_root = add(
        &mut doc,
        None,
        NodeData::Instance(whole_instance(leaf, [20.0, 20.0])),
        10_000.0,
        0.0,
    );
    let outer = ComponentId::new();
    doc.components
        .defs
        .insert(outer, ComponentDef::new(outer, outer_root, "Nested bitmap"));
    let moving = add(
        &mut doc,
        Some(page),
        NodeData::Instance(whole_instance(outer, [20.0, 20.0])),
        -10.0,
        -10.0,
    );
    let resolver = Arc::new(Resolver(std::sync::Mutex::new(crate::DecodedImage::new(
        Arc::new(vec![240, 20, 30, 255]),
        1,
        1,
    ))));
    let inputs = RenderInputs::for_doc(&doc);
    let before = serde_json::to_value(&doc).expect("before");
    let spec = SplitSpec::prepare(&doc.scene, page, moving, &inputs, Some(resolver.as_ref()))
        .expect("nested bitmap plan");
    let viewport = Viewport {
        center: [0.0, 0.0],
        zoom: 1.0,
    };
    let mut renderer = RasterRenderer::new(WIDTH, HEIGHT).expect("renderer");
    renderer.set_asset_resolver(resolver.clone());
    let render = |renderer: &mut RasterRenderer| {
        let mut target = surface();
        for phase in [SplitPhase::Below, SplitPhase::Middle, SplitPhase::Above] {
            renderer
                .paint_split_to_canvas(
                    target.canvas(),
                    WIDTH,
                    HEIGHT,
                    &doc.scene,
                    &viewport,
                    &inputs,
                    &spec,
                    phase,
                )
                .expect("phase");
        }
        pixels(&mut target)
    };
    let initial = render(&mut renderer);
    assert_eq!(rgba_at(&initial, WIDTH, 96, 80), [240, 20, 30, 255]);
    *resolver.0.lock().expect("resolver") =
        crate::DecodedImage::new(Arc::new(vec![20, 240, 30, 255]), 1, 1);
    assert_eq!(render(&mut renderer), initial);
    let mut fresh = RasterRenderer::new(WIDTH, HEIGHT).expect("fresh renderer");
    fresh.set_asset_resolver(resolver);
    let mut changed = surface();
    fresh.render_to_canvas(
        changed.canvas(),
        WIDTH,
        HEIGHT,
        &doc.scene,
        &viewport,
        Some(page),
        &inputs,
    );
    assert_eq!(
        rgba_at(&pixels(&mut changed), WIDTH, 96, 80),
        [20, 240, 30, 255]
    );
    assert_eq!(serde_json::to_value(&doc).expect("after"), before);
}

#[test]
fn split_whole_instances_validate_bindings_after_field_overrides() {
    use fanta_doc::{BoundProp, Override, OverrideValue, VariableId};
    use smallvec::smallvec;
    let (mut doc, page) = instance_page();
    let theme = add_instance_theme(&mut doc);
    doc.active_modes.insert(theme.collection, theme.dark);
    let (component, root) = solid_master(&mut doc, Color::rgb(255, 0, 0));
    doc.scene
        .get_mut(root)
        .expect("master")
        .bindings
        .insert(BoundProp::FillColor { index: 0 }, VariableId::new());
    let mut bound = CanvasNode::new(rectangle(Color::WHITE));
    bound
        .bindings
        .insert(BoundProp::FillColor { index: 0 }, theme.color);
    let bindings = serde_json::to_value(&bound)
        .expect("binding encoding")
        .get("bindings")
        .expect("bindings")
        .clone();
    let mut resolves = whole_instance(component, [20.0, 20.0]);
    resolves.overrides.push(Override {
        target_path: smallvec![],
        target_prop: BoundProp::FillColor { index: 0 },
        value: OverrideValue::Field {
            value: serde_json::json!({"bindings": bindings}),
        },
    });
    let moving = add(
        &mut doc,
        Some(page),
        NodeData::Instance(resolves),
        -40.0,
        -10.0,
    );
    let mut removes = whole_instance(component, [20.0, 20.0]);
    removes.overrides.push(Override {
        target_path: smallvec![],
        target_prop: BoundProp::FillColor { index: 0 },
        value: OverrideValue::Field {
            value: serde_json::json!({"bindings": []}),
        },
    });
    add(
        &mut doc,
        Some(page),
        NodeData::Instance(removes),
        20.0,
        -10.0,
    );
    let pixels = assert_parity_with_inputs(
        &doc,
        page,
        moving,
        &Viewport {
            center: [0.0, 0.0],
            zoom: 1.0,
        },
        None,
        1.0,
        &RenderInputs::for_doc(&doc),
    );
    assert_eq!(
        rgba_at(&pixels, WIDTH, 66, 80),
        [0, 0, 255, 255],
        "resolved override-added binding"
    );
    assert_eq!(
        rgba_at(&pixels, WIDTH, 126, 80),
        [255, 0, 0, 255],
        "override removes unresolved master binding"
    );
}

#[test]
fn split_whole_instances_keep_nested_swapped_derived_sizes_and_text_layout() {
    use fanta_doc::{
        BoundProp, ComponentDef, ComponentId, DerivedOverride, Override, OverrideValue,
    };
    use smallvec::smallvec;
    let (mut doc, page) = instance_page();
    let (original, _) = solid_master(&mut doc, Color::rgb(255, 0, 255));
    let replacement_root = add(
        &mut doc,
        None,
        NodeData::Group(GroupNode {
            clip_size: Some([80.0, 40.0]),
            ..Default::default()
        }),
        10_000.0,
        0.0,
    );
    let child = add(
        &mut doc,
        Some(replacement_root),
        NodeData::Text(TextNode::new("Master text", 20.0, 10.0)),
        0.0,
        0.0,
    );
    let replacement = ComponentId::new();
    doc.components.defs.insert(
        replacement,
        ComponentDef::new(replacement, replacement_root, "Text replacement"),
    );
    let outer_root = add(
        &mut doc,
        None,
        NodeData::Group(GroupNode {
            clip_size: Some([80.0, 40.0]),
            ..Default::default()
        }),
        10_000.0,
        0.0,
    );
    let nested = add(
        &mut doc,
        Some(outer_root),
        NodeData::Instance(whole_instance(original, [80.0, 40.0])),
        0.0,
        0.0,
    );
    let outer = ComponentId::new();
    doc.components
        .defs
        .insert(outer, ComponentDef::new(outer, outer_root, "Derived swap"));
    let mut placed = whole_instance(outer, [80.0, 40.0]);
    placed.overrides.push(Override {
        target_path: smallvec![nested],
        target_prop: BoundProp::Visible,
        value: OverrideValue::SwapInstance {
            component: replacement,
        },
    });
    placed.derived.push(DerivedOverride {
        path: smallvec![nested, child],
        transform: Some(Transform2D::translation(11.0, 7.0)),
        size: Some([65.0, 28.0]),
        fills: None,
        path_data: None,
        stroke_path: None,
        stroke_weight: None,
        text: Some(fanta_doc::node::DerivedText {
            content: Some("Placed text".into()),
            font_size: Some(13.0),
            line_height: Some(1.2),
            line_height_auto_percent: None,
            letter_spacing: None,
            color: Some(Color::WHITE),
            weight: Some(700),
            family: None,
        }),
    });
    let moving = add(
        &mut doc,
        Some(page),
        NodeData::Instance(placed),
        -40.0,
        -20.0,
    );
    let inputs = RenderInputs::for_doc(&doc);
    let spec = SplitSpec::prepare(&doc.scene, page, moving, &inputs, None).expect("derived plan");
    let outer = spec.prepared_instance(moving).expect("prepared outer");
    let nested = outer
        .nodes
        .iter()
        .find(|entry| matches!(entry.node.data, NodeData::Instance(_)))
        .expect("nested placement");
    let nested = spec
        .prepared_instance(nested.node.id)
        .expect("prepared nested");
    let text = nested
        .nodes
        .iter()
        .find(|entry| matches!(entry.node.data, NodeData::Text(_)))
        .expect("derived text");
    let NodeData::Text(payload) = &text.node.data else {
        panic!("text payload")
    };
    assert_eq!(payload.content, "Placed text");
    assert_eq!(payload.local_size, [65.0, 28.0]);
    assert_eq!(text.node.transform, Transform2D::translation(11.0, 7.0));
    for zoom in [0.73, 1.0, 2.0] {
        let pixels = assert_parity_with_inputs(
            &doc,
            page,
            moving,
            &Viewport {
                center: [0.0, 0.0],
                zoom,
            },
            None,
            1.0,
            &RenderInputs::for_doc(&doc),
        );
        assert!(
            pixels
                .chunks_exact(4)
                .any(|pixel| pixel[0] > 100 && pixel[1] > 100 && pixel[2] > 100),
            "resolved text must visibly paint"
        );
        assert!(
            !pixels
                .chunks_exact(4)
                .any(|pixel| pixel[0] > 100 && pixel[1] < 10 && pixel[2] > 100),
            "the swapped-away magenta master must not paint"
        );
    }
}

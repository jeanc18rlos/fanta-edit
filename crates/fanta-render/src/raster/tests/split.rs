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
    let before = serde_json::to_value(doc).expect("before scene");
    let inputs = RenderInputs::empty();
    let spec = SplitSpec::prepare(&doc.scene, page, moving, &inputs, resolver.as_deref())
        .expect("eligible split");
    let mut renderer = RasterRenderer::new(WIDTH, HEIGHT).expect("renderer");
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
        &inputs,
    );
    assert!(!metrics.incomplete_artwork && !metrics.effect_failed);
    let mut actual = surface();
    actual.canvas().clear(skia_safe::Color::TRANSPARENT);
    for phase in [SplitPhase::Below, SplitPhase::Middle, SplitPhase::Above] {
        let mut layer = surface();
        let metrics = renderer
            .render_split_to_canvas(
                layer.canvas(),
                WIDTH,
                HEIGHT,
                &doc.scene,
                viewport,
                &inputs,
                &spec,
                phase,
            )
            .expect("phase");
        assert!(!metrics.incomplete_artwork && !metrics.effect_failed);
        assert_eq!(
            (metrics.layer_cache_hits, metrics.layer_cache_misses),
            (0, 0)
        );
        actual
            .canvas()
            .draw_image(layer.image_snapshot(), (0, 0), None);
    }
    let expected_pixels = pixels(&mut expected);
    let actual_pixels = pixels(&mut actual);
    let differences: Vec<_> = expected_pixels
        .iter()
        .zip(&actual_pixels)
        .map(|(left, right)| left.abs_diff(*right))
        .collect();
    let maximum = differences.iter().copied().max().expect("nonempty image");
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
        &inputs,
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
            3 => node.data.as_group_mut().expect("group").clip_size = Some([100.0, 100.0]),
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

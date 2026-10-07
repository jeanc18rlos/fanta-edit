//! Viewport-culling tests + the `visible_world_rect` inversion unit tests.
use super::*;
use fanta_doc::{Doc, Operation, VectorNode};

// -----------------------------------------------------------------------
// Viewport culling
// -----------------------------------------------------------------------

/// A solid rect of `w × h` whose top-left local corner is at world
/// `(x, y)` (identity transform), so its world AABB is exactly
/// `[x, y, x+w, y+h]`. Returns its `NodeId`.
fn rect_at(doc: &mut Doc, x: f64, y: f64, w: f64, h: f64, color: Color) -> NodeId {
    let n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(x, y, w, h, color)));
    let id = n.id;
    doc.apply(Operation::create_node(n)).unwrap();
    id
}
#[test]
fn visible_world_rect_exactly_inverts_the_render_transform() {
    // zoom=1, display_scale=1, 100x100 surface centred on the origin shows
    // world [-50, 50] on both axes (plus the cull margin).
    let vp = Viewport {
        center: [0.0, 0.0],
        zoom: 1.0,
    };
    let r = visible_world_rect(100, 100, 1.0, &vp);
    assert!(
        (r.min_x - (-50.0 - CULL_MARGIN_WORLD)).abs() < 1e-9,
        "min_x {}",
        r.min_x
    );
    assert!(
        (r.max_x - (50.0 + CULL_MARGIN_WORLD)).abs() < 1e-9,
        "max_x {}",
        r.max_x
    );
    assert!(
        (r.min_y - (-50.0 - CULL_MARGIN_WORLD)).abs() < 1e-9,
        "min_y {}",
        r.min_y
    );
    assert!(
        (r.max_y - (50.0 + CULL_MARGIN_WORLD)).abs() < 1e-9,
        "max_y {}",
        r.max_y
    );
}

#[test]
fn visible_world_rect_tracks_center_and_zoom() {
    // Centre (1000, 0), zoom 2x: a 100px surface shows 50 world units wide,
    // so half-extent is 25, recentred on the pan target. This is what lets a
    // panned viewport un-cull a distant node.
    let vp = Viewport {
        center: [1000.0, 0.0],
        zoom: 2.0,
    };
    let r = visible_world_rect(100, 100, 1.0, &vp);
    assert!(
        (r.min_x - (1000.0 - 25.0 - CULL_MARGIN_WORLD)).abs() < 1e-9,
        "min_x {}",
        r.min_x
    );
    assert!(
        (r.max_x - (1000.0 + 25.0 + CULL_MARGIN_WORLD)).abs() < 1e-9,
        "max_x {}",
        r.max_x
    );
}

#[test]
fn visible_world_rect_disables_culling_on_degenerate_zoom() {
    // A zero/NaN effective scale cannot be inverted; the rect must be
    // all-encompassing so we never wrongly hide the whole scene.
    let vp = Viewport {
        center: [0.0, 0.0],
        zoom: 0.0,
    };
    let r = visible_world_rect(100, 100, 1.0, &vp);
    assert_eq!(r.min_x, f64::NEG_INFINITY);
    assert_eq!(r.max_x, f64::INFINITY);
    // Any finite node intersects it, so nothing is culled.
    let node = Bounds::from_xywh(1e9, 1e9, 10.0, 10.0);
    assert!(node.intersects(&r));
}

#[test]
fn far_offscreen_nodes_are_culled_visible_one_still_renders() {
    let mut doc = Doc::new();
    // One on-screen rect centred at the origin (visible at surface centre).
    rect_at(&mut doc, -10.0, -10.0, 20.0, 20.0, Color::rgb(255, 0, 0));
    // Many rects parked far off-screen, each clearly outside the visible
    // region of a 64x64 surface centred on the origin.
    for i in 0..50 {
        let x = 100_000.0 + (i as f64) * 1_000.0;
        rect_at(&mut doc, x, x, 20.0, 20.0, Color::rgb(0, 255, 0));
    }

    let mut r = RasterRenderer::new(64, 64).unwrap();
    let metrics = r.render(&doc.scene, &doc.viewport);

    assert!(
        metrics.nodes_culled >= 50,
        "all 50 far rects should be culled, got {}",
        metrics.nodes_culled
    );
    assert!(metrics.nodes_drawn >= 1, "the visible rect still draws");

    // The on-screen red rect lit up the centre pixel.
    let buf = r.copy_rgba();
    let c = rgba_at(&buf, 64, 32, 32);
    assert!(c[0] > 200, "centre should be red, got {c:?}");
    assert!(c[3] > 200, "centre should be opaque, got {c:?}");
}

#[test]
fn node_fully_inside_viewport_is_not_culled() {
    let mut doc = Doc::new();
    // A 20x20 rect at the origin sits entirely inside a 64x64 surface's
    // visible world rect (±32). Nothing should be culled.
    rect_at(&mut doc, -10.0, -10.0, 20.0, 20.0, Color::rgb(0, 0, 255));

    let mut r = RasterRenderer::new(64, 64).unwrap();
    let metrics = r.render(&doc.scene, &doc.viewport);

    assert_eq!(
        metrics.nodes_culled, 0,
        "fully-visible node must not be culled"
    );
    assert_eq!(metrics.nodes_visited, 1);
    assert!(metrics.nodes_drawn >= 1);
}

#[test]
fn partially_visible_node_at_the_edge_is_not_culled() {
    let mut doc = Doc::new();
    // 64x64 surface → visible world x ∈ [-32, 32] (+ margin). A 20-wide rect
    // straddling the right edge (x ∈ [25, 45]) overlaps the visible region,
    // so it must be kept, not culled.
    rect_at(&mut doc, 25.0, -10.0, 20.0, 20.0, Color::rgb(255, 0, 0));

    let mut r = RasterRenderer::new(64, 64).unwrap();
    let metrics = r.render(&doc.scene, &doc.viewport);

    assert_eq!(metrics.nodes_culled, 0, "edge-straddling node must be kept");
    assert!(metrics.nodes_drawn >= 1);
    // The left part of the rect (world x ~25 → screen x ~57) is visible.
    let buf = r.copy_rgba();
    let c = rgba_at(&buf, 64, 58, 32);
    assert!(c[0] > 150, "the on-screen sliver should be red, got {c:?}");
}

#[test]
fn offscreen_group_culls_its_entire_subtree_unvisited() {
    use fanta_doc::GroupNode;
    let mut doc = Doc::new();
    // A group translated far off-screen; its world bounds are the union of
    // its children's bounds, so they are off-screen too.
    let mut g = CanvasNode::new(NodeData::Group(GroupNode::default()));
    g.transform = Transform2D::translation(500_000.0, 500_000.0);
    let g_id = g.id;
    doc.apply(Operation::create_node(g)).unwrap();
    // Three children inside the group (group-local coords near origin).
    for _ in 0..3 {
        let mut c = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            10.0,
            10.0,
            Color::rgb(0, 255, 0),
        )));
        c.parent = Some(g_id);
        doc.apply(Operation::create_node(c)).unwrap();
    }

    let mut r = RasterRenderer::new(64, 64).unwrap();
    let metrics = r.render(&doc.scene, &doc.viewport);

    // The group is the single cull decision; its 3 children are never
    // visited (subtree skip), so visited counts only the group.
    assert_eq!(metrics.nodes_culled, 1, "the group is culled once");
    assert_eq!(
        metrics.nodes_visited, 1,
        "the 3 children must not be visited under a culled group"
    );
    assert_eq!(metrics.nodes_drawn, 0, "nothing off-screen is drawn");
    let buf = r.copy_rgba();
    assert!(
        buf.iter().all(|&b| b == 0),
        "off-screen group draws nothing"
    );
}

#[test]
fn panning_the_viewport_unculls_a_distant_node() {
    let mut doc = Doc::new();
    // A single rect far from the origin.
    rect_at(&mut doc, 10_000.0, 0.0, 20.0, 20.0, Color::rgb(255, 0, 0));

    let mut r = RasterRenderer::new(64, 64).unwrap();

    // Default viewport (centred on origin): the distant rect is culled.
    let centred = r.render(&doc.scene, &doc.viewport);
    assert_eq!(
        centred.nodes_culled, 1,
        "distant rect culled from the origin"
    );
    assert_eq!(centred.nodes_drawn, 0);

    // Pan the viewport over the rect: it is now visible and drawn, not culled.
    let panned_vp = Viewport {
        center: [10_010.0, 10.0], // centre of the rect [10000,0]+[20,20]/2
        zoom: 1.0,
    };
    let panned = r.render(&doc.scene, &panned_vp);
    assert_eq!(
        panned.nodes_culled, 0,
        "panned-over rect must not be culled"
    );
    assert!(panned.nodes_drawn >= 1, "and it draws");
    let buf = r.copy_rgba();
    let c = rgba_at(&buf, 64, 32, 32);
    assert!(c[0] > 200, "centre should now be red, got {c:?}");
}

fn scoped_root_fixture() -> Result<(Doc, NodeId, NodeId, NodeId), Box<dyn std::error::Error>> {
    let mut doc = Doc::new();
    let mut parent = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode {
        local_size: Some([1.0, 1.0]),
        clip_size: Some([1.0, 1.0]),
        background: Some(Fill::solid(Color::rgb(255, 0, 0))),
        ..Default::default()
    }));
    parent.transform = Transform2D::translation(40.0, 60.0);
    parent.opacity = fanta_doc::UnitInterval::new(0.25);
    parent.flags.insert(NodeFlags::HIDDEN);
    let parent = doc.scene.insert(parent)?;
    let mut root = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode {
        local_size: Some([160.0, 80.0]),
        clip_size: Some([160.0, 80.0]),
        background: Some(Fill::solid(Color::rgb(0, 0, 255))),
        ..Default::default()
    }));
    root.parent = Some(parent);
    root.transform = Transform2D::translation(200.0, 35.0);
    let root = doc.scene.insert(root)?;
    let mut child = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        20.0,
        12.0,
        Color::rgb(0, 255, 0),
    )));
    child.parent = Some(root);
    child.transform = Transform2D::translation(10.0, 8.0);
    let child = doc.scene.insert(child)?;
    Ok((doc, parent, root, child))
}

fn assert_scoped_root_pixels(
    pixels: &[u8],
    width: u32,
    height: u32,
    background: [u8; 4],
    rectangles: &[([u32; 4], [u8; 4])],
) {
    assert_eq!(pixels.len(), width as usize * height as usize * 4);
    for (offset, pixel) in pixels.chunks_exact(4).enumerate() {
        let x = offset as u32 % width;
        let y = offset as u32 / width;
        let mut expected = background;
        for ([left, top, right, bottom], color) in rectangles {
            if x >= *left && x < *right && y >= *top && y < *bottom {
                expected = *color;
            }
        }
        assert_eq!(pixel, expected.as_slice(), "pixel ({x},{y})");
    }
}

fn scoped_root_surface_pixels(surface: &mut Surface, width: u32, height: u32) -> Vec<u8> {
    let info = ImageInfo::new(
        (width as i32, height as i32),
        ColorType::RGBA8888,
        AlphaType::Unpremul,
        None,
    );
    let mut pixels = vec![0; width as usize * height as usize * 4];
    assert!(surface.read_pixels(&info, &mut pixels, width as usize * 4, (0, 0)));
    pixels
}

#[test]
fn scoped_root_cpu_preserves_world_coordinates_and_scope_isolation()
-> Result<(), Box<dyn std::error::Error>> {
    let (doc, _, root, _) = scoped_root_fixture()?;
    let before = serde_json::to_value(&doc)?;
    let revision = doc.scene.revision();
    let viewport = Viewport {
        center: [320.0, 135.0],
        zoom: 1.0,
    };
    for scale in [1, 2] {
        let mut renderer = RasterRenderer::new(400 * scale, 300 * scale)?;
        renderer.display_scale = f64::from(scale);
        let inputs = RenderInputs::for_doc(&doc);
        let metrics = renderer.render_page_with(&doc.scene, &viewport, Some(root), &inputs);
        assert_eq!(metrics.nodes_visited, 2);
        assert_eq!(metrics.nodes_culled, 0);
        assert_scoped_root_pixels(
            &renderer.copy_rgba(),
            400 * scale,
            300 * scale,
            [0; 4],
            &[
                (
                    [120 * scale, 110 * scale, 280 * scale, 190 * scale],
                    [0, 0, 255, 255],
                ),
                (
                    [130 * scale, 118 * scale, 150 * scale, 130 * scale],
                    [0, 255, 0, 255],
                ),
            ],
        );
        renderer.render_with(&doc.scene, &viewport, &inputs);
        assert!(renderer.copy_rgba().iter().all(|channel| *channel == 0));
    }
    assert_eq!(serde_json::to_value(&doc)?, before);
    assert_eq!(doc.scene.revision(), revision);
    Ok(())
}

#[test]
fn scoped_root_external_canvas_keeps_matrix_and_clip() -> Result<(), Box<dyn std::error::Error>> {
    let (doc, _, root, _) = scoped_root_fixture()?;
    let mut surface = surfaces::raster_n32_premul((400, 300)).ok_or("test surface")?;
    surface.canvas().clear(skia_safe::Color::TRANSPARENT);
    let mut renderer = RasterRenderer::new(400, 300)?;
    let canvas = surface.canvas();
    canvas.translate((3.0, 5.0));
    canvas.clip_rect(Rect::from_xywh(0.0, 0.0, 300.0, 250.0), None, false);
    let matrix = canvas.local_to_device_as_3x3();
    let clip = canvas.device_clip_bounds();
    let saves = canvas.save_count();
    renderer.render_to_canvas(
        canvas,
        400,
        300,
        &doc.scene,
        &Viewport {
            center: [320.0, 135.0],
            zoom: 1.0,
        },
        Some(root),
        &RenderInputs::for_doc(&doc),
    );
    assert_eq!(canvas.local_to_device_as_3x3(), matrix);
    assert_eq!(canvas.device_clip_bounds(), clip);
    assert_eq!(canvas.save_count(), saves);
    assert_scoped_root_pixels(
        &scoped_root_surface_pixels(&mut surface, 400, 300),
        400,
        300,
        [0; 4],
        &[
            ([123, 115, 283, 195], [0, 0, 255, 255]),
            ([133, 123, 153, 135], [0, 255, 0, 255]),
        ],
    );
    Ok(())
}

#[test]
fn scoped_root_tiles_preserve_world_coordinates_and_other_pixels()
-> Result<(), Box<dyn std::error::Error>> {
    let (doc, _, root, _) = scoped_root_fixture()?;
    let inputs = RenderInputs::for_doc(&doc);
    let viewport = Viewport {
        center: [320.0, 135.0],
        zoom: 1.0,
    };
    let background = Color::rgb(17, 19, 23);
    let rectangles = [
        ([17, 23, 417, 323], [255; 4]),
        ([137, 133, 297, 213], [0, 0, 255, 255]),
        ([147, 141, 167, 153], [0, 255, 0, 255]),
    ];
    let mut renderer = RasterRenderer::new(450, 350)?;
    renderer.background = background;
    renderer.render(&Scene::new(), &Viewport::default());
    renderer.render_tile_self(
        17.0,
        23.0,
        400.0,
        300.0,
        Some(Color::WHITE),
        &doc.scene,
        &viewport,
        Some(root),
        &inputs,
    );
    assert_scoped_root_pixels(
        &renderer.copy_rgba(),
        450,
        350,
        [17, 19, 23, 255],
        &rectangles,
    );

    let mut surface = surfaces::raster_n32_premul((450, 350)).ok_or("test surface")?;
    surface.canvas().clear(to_sk_color(background));
    let saves = surface.canvas().save_count();
    let matrix = surface.canvas().local_to_device_as_3x3();
    renderer.render_tile_onto(
        surface.canvas(),
        17.0,
        23.0,
        400.0,
        300.0,
        Some(Color::WHITE),
        &doc.scene,
        &viewport,
        Some(root),
        &inputs,
    );
    assert_eq!(surface.canvas().save_count(), saves);
    assert_eq!(surface.canvas().local_to_device_as_3x3(), matrix);
    assert_scoped_root_pixels(
        &scoped_root_surface_pixels(&mut surface, 450, 350),
        450,
        350,
        [17, 19, 23, 255],
        &rectangles,
    );
    Ok(())
}

#[test]
fn scoped_root_nested_rotation_reflection_and_scale_are_applied_once()
-> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Doc::new();
    let mut grandparent = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode::default()));
    grandparent.transform = Transform2D::from_components([0.0, 1.0, -1.0, 0.0, 500.0, 30.0]);
    let grandparent = doc.scene.insert(grandparent)?;
    let mut parent = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode::default()));
    parent.parent = Some(grandparent);
    parent.transform = Transform2D::from_components([2.0, 0.0, 0.0, -3.0, 40.0, 60.0]);
    let parent = doc.scene.insert(parent)?;
    let (fixture, _, root, child) = scoped_root_fixture()?;
    let mut root_node = fixture.scene.get(root).ok_or("scope root")?.clone();
    root_node.parent = Some(parent);
    doc.scene.insert(root_node)?;
    doc.scene
        .insert(fixture.scene.get(child).ok_or("scope child")?.clone())?;
    let before = serde_json::to_value(&doc)?;
    let mut renderer = RasterRenderer::new(400, 400)?;
    renderer.render_page_with(
        &doc.scene,
        &Viewport {
            center: [665.0, 630.0],
            zoom: 1.0,
        },
        Some(root),
        &RenderInputs::for_doc(&doc),
    );
    // The composed map is (x,y) -> (545+3y,470+2x), independently of Scene's world cache.
    assert_scoped_root_pixels(
        &renderer.copy_rgba(),
        400,
        400,
        [0; 4],
        &[
            ([80, 40, 320, 360], [0, 0, 255, 255]),
            ([104, 60, 140, 100], [0, 255, 0, 255]),
        ],
    );
    assert_eq!(serde_json::to_value(&doc)?, before);
    Ok(())
}

#[test]
fn scoped_root_motion_uses_resolved_ancestor_and_root_transforms()
-> Result<(), Box<dyn std::error::Error>> {
    use fanta_doc::{
        AnimationClipId, MotionEvaluation, MotionProperty, MotionTarget, ResolvedVarValue,
    };
    let (doc, parent, root, _) = scoped_root_fixture()?;
    let before = serde_json::to_value(&doc)?;
    let revision = doc.scene.revision();
    let motion = MotionEvaluation {
        clip: AnimationClipId::new(),
        playhead_ms: 500,
        overrides: BTreeMap::from([
            (
                MotionTarget::new(parent, MotionProperty::PositionX),
                ResolvedVarValue::Float { value: 240.0 },
            ),
            (
                MotionTarget::new(root, MotionProperty::PositionY),
                ResolvedVarValue::Float { value: 45.0 },
            ),
        ]),
    };
    let mut inputs = RenderInputs::for_doc(&doc);
    inputs.motion = Some(&motion);
    let mut renderer = RasterRenderer::new(400, 300)?;
    let metrics = renderer.render_page_with(
        &doc.scene,
        &Viewport {
            center: [520.0, 145.0],
            zoom: 1.0,
        },
        Some(root),
        &inputs,
    );
    assert_eq!(metrics.nodes_culled, 0);
    assert_scoped_root_pixels(
        &renderer.copy_rgba(),
        400,
        300,
        [0; 4],
        &[
            ([120, 110, 280, 190], [0, 0, 255, 255]),
            ([130, 118, 150, 130], [0, 255, 0, 255]),
        ],
    );
    assert_eq!(serde_json::to_value(&doc)?, before);
    assert_eq!(doc.scene.revision(), revision);
    Ok(())
}

#[test]
fn scoped_root_without_parent_and_missing_scope_keep_existing_behavior()
-> Result<(), Box<dyn std::error::Error>> {
    let (fixture, _, root, child) = scoped_root_fixture()?;
    let mut doc = Doc::new();
    let mut root_node = fixture.scene.get(root).ok_or("scope root")?.clone();
    root_node.parent = None;
    root_node.transform = Transform2D::translation(240.0, 95.0);
    doc.scene.insert(root_node)?;
    doc.scene
        .insert(fixture.scene.get(child).ok_or("scope child")?.clone())?;
    let viewport = Viewport {
        center: [320.0, 135.0],
        zoom: 1.0,
    };
    let mut renderer = RasterRenderer::new(400, 300)?;
    for scope in [None, Some(root)] {
        renderer.render_page_with(&doc.scene, &viewport, scope, &RenderInputs::for_doc(&doc));
        assert_scoped_root_pixels(
            &renderer.copy_rgba(),
            400,
            300,
            [0; 4],
            &[
                ([120, 110, 280, 190], [0, 0, 255, 255]),
                ([130, 118, 150, 130], [0, 255, 0, 255]),
            ],
        );
    }
    renderer.render_page_with(
        &doc.scene,
        &viewport,
        Some(NodeId::new()),
        &RenderInputs::for_doc(&doc),
    );
    assert!(renderer.copy_rgba().iter().all(|channel| *channel == 0));
    Ok(())
}

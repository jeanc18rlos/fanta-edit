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

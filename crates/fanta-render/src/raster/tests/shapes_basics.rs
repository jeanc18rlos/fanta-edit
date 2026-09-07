//! Basic shape rendering: renderer construction, empty-scene clearing, a
//! centered solid rect, uniform + per-corner rounding, and auto-layout
//! reverse-z paint order.
use super::*;
use fanta_doc::{AutoLayout, Doc, GroupNode, Operation, VectorNode};

#[test]
fn renderer_construction_succeeds() {
    let r = RasterRenderer::new(64, 64);
    assert!(r.is_ok());
}

#[test]
fn empty_scene_yields_transparent_pixels() {
    let mut r = RasterRenderer::new(16, 16).unwrap();
    let doc = Doc::new();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();
    // Skia n32_premul is BGRA on little-endian platforms — but for a
    // cleared-to-transparent surface every byte is zero.
    assert!(buf.iter().all(|&b| b == 0), "expected transparent canvas");
}

#[test]
fn red_rect_centered_lights_up_center_pixel() {
    let mut r = RasterRenderer::new(64, 64).unwrap();
    let doc = red_rect_doc(20.0, 20.0);
    let metrics = r.render(&doc.scene, &doc.viewport);
    assert!(metrics.nodes_drawn >= 1);
    let buf = r.copy_rgba();
    // copy_rgba requests RGBA8888 explicitly, so byte 0 is red.
    let row_bytes = (r.width() * 4) as usize;
    let cx = r.width() as usize / 2;
    let cy = r.height() as usize / 2;
    let i = cy * row_bytes + cx * 4;
    let red = buf[i];
    let g = buf[i + 1];
    let b = buf[i + 2];
    let a = buf[i + 3];
    assert!(red > 200, "expected red channel high, got {red}");
    assert!(g < 30, "expected green channel low, got {g}");
    assert!(b < 30, "expected blue channel low, got {b}");
    assert!(a > 200, "expected alpha opaque, got {a}");
}

#[test]
fn rounded_rect_clips_its_corner() {
    // A vector rect with corner_radius == half its size is a "pill"/circle;
    // its extreme corner pixel must fall outside the rounding and stay
    // transparent, while the center stays filled. This proves the importer's
    // corner_radius reaches the renderer as an actual rounded shape (Figma
    // buttons/badges) rather than a square box.
    let mut doc = Doc::new();
    let mut n = CanvasNode::new(NodeData::Vector(VectorNode {
        path: fanta_doc::PathData::rect(-20.0, -20.0, 40.0, 40.0),
        fills: smallvec_of(Fill::solid(Color::rgb(255, 0, 0))),
        strokes: Default::default(),
        corner_radius: Some(20.0),
        corner_radii: None,
        corner_smoothing: 0.0,
        local_size: None,
        parametric: None,
    }));
    n.name = "pill".into();
    doc.apply(Operation::create_node(n)).unwrap();

    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();
    let row_bytes = (r.width() * 4) as usize;
    let px = |x: usize, y: usize| {
        let i = y * row_bytes + x * 4;
        (buf[i], buf[i + 3]) // (red, alpha)
    };
    // Center filled red.
    let (cr, ca) = px(r.width() as usize / 2, r.height() as usize / 2);
    assert!(cr > 200 && ca > 200, "center should be opaque red");
    // Extreme top-left corner of the rect's bounding box is rounded away.
    // The rect spans world (-20,-20)..(20,20); at zoom 1, centered, that is
    // pixels (12,12)..(52,52). The (12,12) corner is outside the radius.
    let (_r0, a0) = px(13, 13);
    assert!(
        a0 < 40,
        "rounded corner pixel must be (near) transparent, got alpha {a0}"
    );
}

#[test]
fn independent_corner_radii_round_distinct_corners() {
    // A rect rounded ONLY at top-left (big radius) and square elsewhere must
    // clip its TL corner away while keeping the TR corner filled — proving
    // `corner_radii` ([TL,TR,BR,BL]) builds a per-corner RRect, not a uniform
    // round-rect. This is the mixed-corner card/tab/segmented-control case.
    let mut doc = Doc::new();
    let mut n = CanvasNode::new(NodeData::Vector(VectorNode {
        path: fanta_doc::PathData::rect(-20.0, -20.0, 40.0, 40.0),
        fills: smallvec_of(Fill::solid(Color::rgb(255, 0, 0))),
        strokes: Default::default(),
        corner_radius: None,
        corner_radii: Some([20.0, 0.0, 0.0, 0.0]), // round only TL
        corner_smoothing: 0.0,
        local_size: None,
        parametric: None,
    }));
    n.name = "tab".into();
    doc.apply(Operation::create_node(n)).unwrap();

    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();
    let row_bytes = (r.width() * 4) as usize;
    let alpha = |x: usize, y: usize| buf[y * row_bytes + x * 4 + 3];
    // Rect spans screen pixels (12,12)..(52,52). TL corner (13,13) is rounded
    // away; TR corner (50,13) is a square corner and stays filled.
    assert!(alpha(13, 13) < 40, "rounded TL corner must be transparent");
    assert!(alpha(50, 13) > 200, "square TR corner must stay filled");
    assert!(alpha(50, 50) > 200, "square BR corner must stay filled");
}

#[test]
fn auto_layout_reverse_z_paints_children_in_reverse_order() {
    let mut doc = Doc::new();
    let mut parent = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([40.0, 40.0]),
        auto_layout: Some(AutoLayout {
            reverse_z: true,
            ..Default::default()
        }),
        ..Default::default()
    }));
    parent.transform = Transform2D::translation(-20.0, -20.0);
    let parent_id = parent.id;
    doc.apply(Operation::create_node(parent)).unwrap();

    let red = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        40.0,
        40.0,
        Color::rgb(255, 0, 0),
    )));
    let red_id = red.id;
    doc.apply(Operation::create_node(red)).unwrap();
    let red_index = doc.scene.next_child_index(Some(parent_id));
    doc.scene
        .set_parent(red_id, Some(parent_id), red_index)
        .unwrap();

    let blue = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        40.0,
        40.0,
        Color::rgb(0, 0, 255),
    )));
    let blue_id = blue.id;
    doc.apply(Operation::create_node(blue)).unwrap();
    let blue_index = doc.scene.next_child_index(Some(parent_id));
    doc.scene
        .set_parent(blue_id, Some(parent_id), blue_index)
        .unwrap();

    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();
    let center = rgba_at(&buf, 64, 32, 32);
    assert!(
        center[0] > 200 && center[2] < 40,
        "reverse_z should paint the first child over the later sibling, got {center:?}"
    );
}

#[test]
fn path_cache_populates_hits_evicts_and_clears() {
    let mut r = RasterRenderer::new(64, 64).unwrap();
    let mut doc = red_rect_doc(20.0, 20.0);
    assert_eq!(r.path_cache_len(), 0, "starts empty");
    r.render(&doc.scene, &doc.viewport);
    assert_eq!(r.path_cache_len(), 1, "one visible vector cached");
    let first = r.copy_rgba();
    r.render(&doc.scene, &doc.viewport);
    assert_eq!(
        r.path_cache_len(),
        1,
        "cache hit — no growth on a re-render"
    );
    assert_eq!(
        first,
        r.copy_rgba(),
        "a cache-hit frame must be pixel-identical to the built frame"
    );

    // An edit bumps the scene revision: the next render evicts the stale
    // entries and re-caches the visible vectors under the new revision.
    let green = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -30.0,
        -30.0,
        10.0,
        10.0,
        Color::rgb(0, 255, 0),
    )));
    doc.apply(Operation::create_node(green)).unwrap();
    r.render(&doc.scene, &doc.viewport);
    assert_eq!(
        r.path_cache_len(),
        2,
        "stale-revision entry evicted; both vectors cached at the new revision"
    );

    r.clear_path_cache();
    assert_eq!(r.path_cache_len(), 0, "clear empties it");
    r.render(&doc.scene, &doc.viewport);
    assert!(r.path_cache_len() > 0, "re-render repopulates");
}

#[test]
fn path_cache_does_not_serve_stale_geometry_across_scene_instances() {
    // A reloaded/reparsed document is a NEW Scene instance whose revision
    // counter restarts, while node ids persist by design (stable-id sidecar
    // pairing). A structurally-identical reparse therefore lands on exactly
    // the (NodeId, revision) cache keys of the scene it replaced — with
    // different geometry. The renderer's instance-id guard must clear the
    // revision-keyed caches instead of serving the first scene's outline.
    let mut renderer = RasterRenderer::new(64, 64).unwrap();
    let viewport = Viewport::default();

    let mut first = Scene::new();
    let big = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -20.0,
        -20.0,
        40.0,
        40.0,
        Color::rgb(255, 0, 0),
    )));
    let shared_id = big.id;
    first.insert(big).unwrap();

    let mut second = Scene::new();
    let mut small = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -5.0,
        -5.0,
        10.0,
        10.0,
        Color::rgb(255, 0, 0),
    )));
    small.id = shared_id;
    second.insert(small).unwrap();

    assert_eq!(
        first.revision(),
        second.revision(),
        "precondition: the reparse collides on the same revision"
    );
    assert_ne!(
        first.instance_id(),
        second.instance_id(),
        "precondition: distinct scene instances carry distinct ids"
    );

    renderer.render(&first, &viewport);
    let buf = renderer.copy_rgba();
    // (47, 32) is 15 world units right of center: inside the 40x40 rect,
    // well outside the 10x10 one.
    assert!(
        rgba_at(&buf, 64, 47, 32)[3] > 200,
        "the big rect covers the probe pixel"
    );
    assert_eq!(renderer.path_cache_len(), 1, "first scene's path is cached");

    renderer.render(&second, &viewport);
    let buf = renderer.copy_rgba();
    let center = rgba_at(&buf, 64, 32, 32);
    assert!(
        center[0] > 200 && center[3] > 200,
        "the small rect still fills the center, got {center:?}"
    );
    let probe = rgba_at(&buf, 64, 47, 32);
    assert_eq!(
        probe[3], 0,
        "probe pixel outside the second scene's rect must be transparent — \
         opaque means the first scene's cached outline was served, got {probe:?}"
    );
}

#[test]
fn geometry_stamps_rebuild_only_the_edited_node() {
    let mut r = RasterRenderer::new(64, 64).unwrap();
    let mut doc = red_rect_doc(20.0, 20.0);
    let green = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -30.0,
        -30.0,
        10.0,
        10.0,
        Color::rgb(0, 255, 0),
    )));
    let green_id = green.id;
    doc.apply(Operation::create_node(green)).unwrap();

    let cold = r.render(&doc.scene, &doc.viewport);
    assert_eq!(
        cold.paths_built, 2,
        "cold frame builds every visible vector"
    );
    let warm = r.render(&doc.scene, &doc.viewport);
    assert_eq!(warm.paths_built, 0, "steady frame is all cache hits");

    // Transform-only write through the scoped mutator: no geometry stamp,
    // no rebuild — the drag case.
    doc.scene
        .set_transform(green_id, fanta_doc::Transform2D::translation(1.0, 0.0))
        .unwrap();
    let dragged = r.render(&doc.scene, &doc.viewport);
    assert_eq!(
        dragged.paths_built, 0,
        "a transform-only edit must not rebuild any cached path"
    );

    // A conservative mutable access stamps that node — and only that node.
    doc.scene.get_mut(green_id).unwrap();
    let after_touch = r.render(&doc.scene, &doc.viewport);
    assert_eq!(
        after_touch.paths_built, 1,
        "touching one node rebuilds exactly that node's paths"
    );

    // The cache survives a scene-instance swap (the session rebuilds its
    // projection as a fresh clone after every edit).
    let clone = doc.scene.clone();
    let cloned_frame = r.render(&clone, &doc.viewport);
    assert_eq!(
        cloned_frame.paths_built, 0,
        "an unchanged clone serves every path from the warm cache"
    );

    // Removal purges the dead entry (gated on the removal revision).
    let snapshot: Vec<CanvasNode> = doc
        .scene
        .descendants_of(green_id)
        .filter_map(|id| doc.scene.get(id).cloned())
        .collect();
    doc.apply(Operation::DeleteSubtree { snapshot }).unwrap();
    r.render(&doc.scene, &doc.viewport);
    assert_eq!(
        r.path_cache_len(),
        1,
        "the removed node's entry is purged; the survivor stays cached"
    );
}

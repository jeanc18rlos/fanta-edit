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

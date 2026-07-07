//! Frame/group rendering: background fill(s), border stroke, rounded-frame
//! corner clipping, and content overflow clipping (incl. nested clip stacks).
use super::*;
use fanta_doc::{Doc, GroupNode, Operation, VectorNode};

#[test]
fn frame_group_paints_its_background_fill() {
    // A group carrying a clip_size + solid background (a Figma FRAME) must
    // paint that background rect. Regression for "frame content floats on
    // the dark canvas" — without it, the center pixel would be the cleared
    // (transparent) background instead of the frame color.
    let mut doc = Doc::new();
    let mut frame = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode {
        clip_size: Some([40.0, 40.0]),
        background: Some(Fill::solid(Color::rgb(0, 200, 0))),
        explicit_modes: Default::default(),
        ..Default::default()
    }));
    // Center the [0,0,40,40] local box on the world origin.
    frame.transform = Transform2D::translation(-20.0, -20.0);
    doc.apply(Operation::create_node(frame)).unwrap();

    let mut r = RasterRenderer::new(64, 64).unwrap();
    let metrics = r.render(&doc.scene, &doc.viewport);
    assert!(
        metrics.nodes_drawn >= 1,
        "frame background should count as a draw"
    );
    let buf = r.copy_rgba();
    let row_bytes = (r.width() * 4) as usize;
    let i = (r.height() as usize / 2) * row_bytes + (r.width() as usize / 2) * 4;
    assert!(
        buf[i] < 40 && buf[i + 1] > 180 && buf[i + 2] < 40 && buf[i + 3] > 200,
        "center pixel should be the green frame background, got {:?}",
        &buf[i..i + 4]
    );
}

#[test]
fn frame_group_paints_stacked_background_fills_under_border() {
    let mut doc = Doc::new();
    let mut stroke = fanta_doc::Stroke::solid(Color::rgb(0, 0, 0), 4.0);
    stroke.align = fanta_doc::StrokeAlign::Inside;
    let mut frame = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode {
        clip_size: Some([40.0, 40.0]),
        background: Some(Fill::solid(Color::rgb(255, 0, 0))),
        background_fills: smallvec::smallvec![Fill::solid(Color::rgb(0, 0, 255))],
        strokes: smallvec::smallvec![stroke],
        ..Default::default()
    }));
    frame.transform = Transform2D::translation(-20.0, -20.0);
    doc.apply(Operation::create_node(frame)).unwrap();

    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    let center = rgba_at(&buf, 64, 32, 32);
    assert!(
        center[2] > 200 && center[3] > 200,
        "top background fill should paint blue at centre, got {center:?}"
    );
    let edge = rgba_at(&buf, 64, 14, 32);
    assert!(
        edge[0] < 40 && edge[1] < 40 && edge[2] < 40 && edge[3] > 200,
        "inside border should stay above stacked fills, got {edge:?}"
    );
}

#[test]
fn frame_group_strokes_its_border() {
    // A frame (clip_size, NO background) carrying an Inside-aligned blue
    // border must paint border pixels on its box edge and leave the interior
    // transparent. Regression for "frame/section borders missing entirely".
    //
    // Coordinate map (64×64, origin-centred, zoom 1): the frame's
    // translate(-20,-20) centres its [0,40] local box on the world origin, so
    // local (0,0)→screen (12,12) and local (40,40)→screen (52,52); the box
    // centre is screen (32,32). The 4px inside border hugs the left edge at
    // screen x ∈ [12, 16].
    let mut doc = Doc::new();
    let mut stroke = fanta_doc::Stroke::solid(Color::rgb(0, 0, 255), 4.0);
    stroke.align = fanta_doc::StrokeAlign::Inside;
    let mut strokes = smallvec::SmallVec::new();
    strokes.push(stroke);
    let mut frame = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode {
        clip_size: Some([40.0, 40.0]),
        background: None,
        strokes,
        ..Default::default()
    }));
    frame.transform = Transform2D::translation(-20.0, -20.0);
    doc.apply(Operation::create_node(frame)).unwrap();

    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();
    // Just inside the left edge → blue border.
    let edge = rgba_at(&buf, 64, 14, 32);
    assert!(
        edge[2] > 200 && edge[3] > 200,
        "left frame edge should be the blue border, got {edge:?}"
    );
    // The frame centre (no background) → transparent.
    let center = rgba_at(&buf, 64, 32, 32);
    assert!(
        center[3] < 40,
        "borderless interior should be transparent, got {center:?}"
    );
}

#[test]
fn rounded_frame_inside_border_keeps_top_edge_opaque() {
    let mut doc = Doc::new();
    let mut stroke = fanta_doc::Stroke::solid(Color::BLACK, 2.0);
    stroke.align = fanta_doc::StrokeAlign::Inside;
    let mut frame = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode {
        clip_size: Some([40.0, 40.0]),
        background: Some(Fill::solid(Color::WHITE)),
        strokes: smallvec::smallvec![stroke],
        corner_radius: Some(8.0),
        ..Default::default()
    }));
    frame.transform = Transform2D::translation(-20.0, -20.0);
    doc.apply(Operation::create_node(frame)).unwrap();

    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    let top = rgba_at(&buf, 64, 32, 12);
    assert!(
        top[0] < 80 && top[1] < 80 && top[2] < 80 && top[3] > 220,
        "rounded inside border top edge should stay opaque black, got {top:?}"
    );
}

#[test]
fn figma_section_does_not_paint_its_outline_as_document_content() {
    let mut doc = Doc::new();
    let mut stroke = fanta_doc::Stroke::solid(Color::BLACK, 4.0);
    stroke.align = fanta_doc::StrokeAlign::Inside;
    let mut section = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode {
        clip_size: Some([40.0, 40.0]),
        background: Some(Fill::solid(Color::rgb(0, 200, 0))),
        strokes: smallvec::smallvec![stroke],
        ..Default::default()
    }));
    section.meta["figma_type"] = serde_json::Value::String("SECTION".to_owned());
    section.transform = Transform2D::translation(-20.0, -20.0);
    doc.apply(Operation::create_node(section)).unwrap();

    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    let edge = rgba_at(&buf, 64, 14, 32);
    assert!(
        edge[0] < 40 && edge[1] > 180 && edge[2] < 40 && edge[3] > 200,
        "section edge should be its background, not an imported outline, got {edge:?}"
    );
}

#[test]
fn frame_group_without_stroke_draws_no_border() {
    // A frame with a background but NO stroke must not gain a phantom border:
    // the whole box (edge included) is the background color, nothing else.
    let mut doc = Doc::new();
    let mut frame = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode {
        clip_size: Some([40.0, 40.0]),
        background: Some(Fill::solid(Color::rgb(0, 200, 0))),
        ..Default::default()
    }));
    frame.transform = Transform2D::translation(-20.0, -20.0);
    doc.apply(Operation::create_node(frame)).unwrap();

    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();
    // Edge and centre are both the green background — no stroke band.
    for (x, y) in [(14, 32), (32, 32)] {
        let p = rgba_at(&buf, 64, x, y);
        assert!(
            p[0] < 40 && p[1] > 180 && p[2] < 40 && p[3] > 200,
            "({x},{y}) should be green background with no border, got {p:?}"
        );
    }
}

#[test]
fn rounded_frame_clips_its_corner() {
    // A frame with a corner radius == half its size is a "pill"; its rounded
    // background must leave the box CORNER transparent (the rounding cut it
    // off) while the centre stays filled. Proves the frame background honors
    // corner radius rather than drawing a square.
    let mut doc = Doc::new();
    let mut frame = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode {
        clip_size: Some([40.0, 40.0]),
        background: Some(Fill::solid(Color::rgb(0, 200, 0))),
        corner_radius: Some(20.0),
        ..Default::default()
    }));
    frame.transform = Transform2D::translation(-20.0, -20.0);
    doc.apply(Operation::create_node(frame)).unwrap();

    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();
    // Top-left box corner (screen ~13,13) is outside the rounded shape →
    // transparent; the centre (32,32) is filled green.
    let corner = rgba_at(&buf, 64, 13, 13);
    assert!(
        corner[3] < 40,
        "rounded frame corner should be transparent (not a square fill), got {corner:?}"
    );
    let center = rgba_at(&buf, 64, 32, 32);
    assert!(
        center[1] > 180 && center[3] > 200,
        "rounded frame centre should be green, got {center:?}"
    );
}

/// Build a doc with a Figma-style frame group whose local `[0,0,40,40]` box
/// is centred on the world origin, holding one red child rect that overflows
/// the box on the right (child local x ∈ [10, 70], so x ∈ (40, 70] spills
/// past the frame edge). `clip` toggles the frame's `clip_size`:
/// `Some([40,40])` (a frame, should clip) vs `None` (a plain group, no clip).
///
/// Coordinate map (100×100 surface, default origin-centred viewport, zoom 1,
/// display_scale 1): world→screen adds 50; the frame's translate(-20,-20)
/// makes local→world subtract 20. So local (20,20) → screen (50,50) is the
/// frame centre (inside both child and frame), and local (50,20) →
/// screen (80,50) is inside the child's unclipped extent but OUTSIDE the
/// frame box — the pixel that proves whether clipping happened.
fn frame_with_overflowing_child_doc(clip: bool, clip_content: Option<bool>) -> Doc {
    let mut doc = Doc::new();
    let mut frame = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode {
        clip_size: if clip { Some([40.0, 40.0]) } else { None },
        // A distinct (blue) background so a clipped out-of-box pixel can't be
        // mistaken for the child's red; the child is what must get clipped.
        background: Some(Fill::solid(Color::rgb(0, 0, 200))),
        explicit_modes: Default::default(),
        ..Default::default()
    }));
    if let Some(clip_content) = clip_content {
        frame.meta["clip_content"] = serde_json::Value::Bool(clip_content);
    }
    frame.transform = Transform2D::translation(-20.0, -20.0);
    let frame_id = frame.id;
    doc.apply(Operation::create_node(frame)).unwrap();

    // Child rect in frame-local space: x ∈ [10, 70], y ∈ [10, 30]. Its right
    // half (x > 40) overflows the frame's [0,40] box.
    let mut child = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        10.0,
        10.0,
        60.0,
        20.0,
        Color::rgb(255, 0, 0),
    )));
    child.parent = Some(frame_id);
    doc.apply(Operation::create_node(child)).unwrap();
    doc
}

#[test]
fn frame_clip_clips_child_overflow_to_the_frame_box() {
    // A frame (clip_size = Some) must confine its child to the frame box: a
    // pixel inside the child's unclipped extent but OUTSIDE the box must NOT
    // be the child's red (it was clipped away), while a pixel inside both
    // still shows the child's red.
    let doc = frame_with_overflowing_child_doc(true, None);
    let mut r = RasterRenderer::new(100, 100).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    // Inside both child and frame → the child's red shows through.
    let inside = rgba_at(&buf, 100, 50, 50);
    assert!(
        inside[0] > 200 && inside[1] < 40 && inside[2] < 40,
        "pixel inside both child and frame should be the child's red, got {inside:?}"
    );

    // Inside the child's unclipped extent but outside the frame box: clipped,
    // so it must NOT be red (here it is outside the background too, hence the
    // cleared transparent canvas — the key assertion is "not the child").
    let clipped = rgba_at(&buf, 100, 80, 50);
    assert!(
        !(clipped[0] > 200 && clipped[1] < 40 && clipped[2] < 40),
        "pixel outside the frame box must be clipped (not the child's red), got {clipped:?}"
    );
}

#[test]
fn plain_group_does_not_clip_oversized_child() {
    // A plain group (clip_size = None) must NOT clip: the same child that the
    // frame clipped now paints fully, so the out-of-box pixel shows red.
    let doc = frame_with_overflowing_child_doc(false, None);
    let mut r = RasterRenderer::new(100, 100).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    // Same sample as the clipped case (screen (80,50)); with no clip the
    // child's red reaches here.
    let overflow = rgba_at(&buf, 100, 80, 50);
    assert!(
        overflow[0] > 200 && overflow[1] < 40 && overflow[2] < 40,
        "with no clip the oversized child should still paint here, got {overflow:?}"
    );
}

#[test]
fn clip_size_with_clip_content_false_does_not_clip_oversized_child() {
    // Figma SECTION nodes have a box/background but `frameMaskDisabled=true`:
    // they carry a clip_size for their own geometry, while descendants can
    // overflow visibly past that box.
    let doc = frame_with_overflowing_child_doc(true, Some(false));
    let mut r = RasterRenderer::new(100, 100).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    let overflow = rgba_at(&buf, 100, 80, 50);
    assert!(
        overflow[0] > 200 && overflow[1] < 40 && overflow[2] < 40,
        "clip_content=false should let the oversized child paint past clip_size, got {overflow:?}"
    );
}

#[test]
fn nested_frame_clip_accumulates_with_ancestor_clip() {
    let mut doc = Doc::new();

    let mut parent = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([40.0, 40.0]),
        background: Some(Fill::solid(Color::rgb(0, 0, 200))),
        explicit_modes: Default::default(),
        ..Default::default()
    }));
    parent.transform = Transform2D::translation(-20.0, -20.0);
    let parent_id = parent.id;
    doc.apply(Operation::create_node(parent)).unwrap();

    let mut child_frame = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([20.0, 20.0]),
        background: Some(Fill::solid(Color::rgb(0, 0, 200))),
        explicit_modes: Default::default(),
        ..Default::default()
    }));
    child_frame.parent = Some(parent_id);
    child_frame.transform = Transform2D::translation(50.0, 10.0);
    let child_id = child_frame.id;
    doc.apply(Operation::create_node(child_frame)).unwrap();

    let mut child_content = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        10.0,
        20.0,
        Color::rgb(255, 0, 0),
    )));
    child_content.parent = Some(child_id);
    doc.apply(Operation::create_node(child_content)).unwrap();

    let mut r = RasterRenderer::new(110, 100).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    let clipped_descendant = rgba_at(&buf, 110, 90, 50);
    assert!(
        !(clipped_descendant[0] > 200 && clipped_descendant[1] < 40 && clipped_descendant[2] < 40),
        "nested frame clips should accumulate with the ancestor, got {clipped_descendant:?}"
    );

    let clipped_child_background = rgba_at(&buf, 110, 102, 50);
    assert!(
        !(clipped_child_background[2] > 150 && clipped_child_background[0] < 40),
        "child frame background itself should still be clipped by the ancestor, got {clipped_child_background:?}"
    );
}

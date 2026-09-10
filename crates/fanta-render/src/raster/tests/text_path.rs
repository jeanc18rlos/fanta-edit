use super::*;
use fanta_doc::{Doc, GroupNode, Operation, PathData, TextPathNode, TextStyleRun, Transform2D};

fn straight_text_path(content: &str) -> TextPathNode {
    let mut path = PathData::new();
    path.move_to(-100.0, 12.0).line_to(100.0, 12.0);
    let mut text_path = TextPathNode::new(path, content);
    text_path.style.size_px = 42.0;
    text_path.style.color = Color::rgb(20, 40, 220);
    text_path
}

fn insert_text_path(doc: &mut Doc, text_path: TextPathNode) -> NodeId {
    let node = CanvasNode::new(NodeData::TextPath(text_path));
    let id = node.id;
    doc.apply(Operation::create_node(node)).unwrap();
    id
}

#[test]
fn text_path_draws_real_shaped_glyphs() {
    let mut doc = Doc::new();
    insert_text_path(&mut doc, straight_text_path("Fanta"));
    let mut renderer = RasterRenderer::new(280, 140).unwrap();
    let metrics = renderer.render(&doc.scene, &doc.viewport);
    let pixels = renderer.copy_rgba();

    assert!(metrics.nodes_drawn >= 1);
    assert!(opaque_pixel_count(&pixels) > 100);
    assert!(
        pixels
            .chunks_exact(4)
            .any(|pixel| { pixel[3] > 120 && pixel[2] > pixel[0] && pixel[2] > pixel[1] })
    );
}

#[test]
fn text_path_style_runs_keep_distinct_glyph_colors() {
    let mut text_path = straight_text_path("AB");
    let mut red = text_path.style.clone();
    red.color = Color::rgb(230, 20, 20);
    text_path.style_runs.push(TextStyleRun {
        start: 1,
        end: 2,
        style: red,
    });
    let mut doc = Doc::new();
    insert_text_path(&mut doc, text_path);
    let mut renderer = RasterRenderer::new(280, 140).unwrap();
    renderer.render(&doc.scene, &doc.viewport);
    let pixels = renderer.copy_rgba();

    let blue = pixels
        .chunks_exact(4)
        .any(|pixel| pixel[3] > 120 && pixel[2] > 140 && pixel[0] < 100);
    let red = pixels
        .chunks_exact(4)
        .any(|pixel| pixel[3] > 120 && pixel[0] > 140 && pixel[2] < 100);
    assert!(
        blue && red,
        "base and override colors both reach glyph paint"
    );
}

#[test]
fn text_path_outline_is_the_effect_silhouette_and_layer_bound() {
    let text_path = straight_text_path("Effects");
    let node = CanvasNode::new(NodeData::TextPath(text_path.clone()));
    let scene = Scene::new();
    let exact = text_path_bounds(&text_path).expect("glyph bounds");
    let layer = effects_layer_bounds(&node, None, &scene).expect("effect layer bounds");
    assert_eq!(layer, exact);

    let silhouette = crate::raster::effects::node_silhouette_path(&node, None, &scene)
        .expect("glyph silhouette");
    let silhouette_bounds = silhouette.compute_tight_bounds();
    assert!((f64::from(silhouette_bounds.left) - exact.min_x).abs() < 0.01);
    assert!((f64::from(silhouette_bounds.top) - exact.min_y).abs() < 0.01);
    assert!((f64::from(silhouette_bounds.right) - exact.max_x).abs() < 0.01);
    assert!((f64::from(silhouette_bounds.bottom) - exact.max_y).abs() < 0.01);
}

#[test]
fn text_path_visual_bounds_include_glyphs_decorations_and_effect_reach() {
    let mut text_path = straight_text_path("Bounds");
    text_path.style.underline = true;
    text_path.style.strikethrough = true;
    let glyph_bounds = text_path_bounds(&text_path).expect("decorated glyph bounds");

    let mut node = CanvasNode::new(NodeData::TextPath(text_path));
    node.transform = Transform2D::translation(30.0, -8.0);
    node.effects.push(Shadow {
        kind: ShadowKind::Drop,
        color: Color::rgba(0, 0, 0, 220),
        blur: 12.0,
        spread: 2.0,
        offset: [18.0, 10.0],
        show_behind_node: false,
    });
    let id = node.id;
    let mut doc = Doc::new();
    doc.apply(Operation::create_node(node)).unwrap();

    let visual = crate::visual_world_bounds(&doc.scene, id, 0.0).expect("visual export bounds");
    let transformed = glyph_bounds
        .try_transformed(&Transform2D::translation(30.0, -8.0))
        .expect("finite transform");
    assert!(visual.min_x < transformed.min_x);
    assert!(visual.min_y < transformed.min_y);
    assert!(visual.max_x > transformed.max_x);
    assert!(visual.max_y > transformed.max_y);
}

#[test]
fn unclipped_group_culling_uses_exact_text_path_descendant_bounds() {
    let mut path = PathData::new();
    path.move_to(-20.0, 80.0).line_to(100.0, 80.0);
    let mut text_path = TextPathNode::new(path, "H");
    text_path.style.size_px = 64.0;
    text_path.style.color = Color::rgb(20, 40, 220);

    let mut doc = Doc::new();
    let outer = CanvasNode::new(NodeData::Group(GroupNode::default()));
    let outer_id = outer.id;
    doc.apply(Operation::create_node(outer)).unwrap();
    let mut inner = CanvasNode::new(NodeData::Group(GroupNode::default()));
    let inner_id = inner.id;
    inner.parent = Some(outer_id);
    doc.apply(Operation::create_node(inner)).unwrap();
    let mut child = CanvasNode::new(NodeData::TextPath(text_path));
    child.parent = Some(inner_id);
    doc.apply(Operation::create_node(child)).unwrap();

    let authored = doc
        .scene
        .local_bounds(outer_id)
        .expect("baseline bounds are cached");
    assert!(authored.min_y > 52.0);

    let mut renderer = RasterRenderer::new(100, 100).unwrap();
    renderer.render(&doc.scene, &doc.viewport);
    assert!(opaque_pixel_count(&renderer.copy_rgba()) > 0);
}

#[test]
fn page_and_png_export_paths_share_text_path_rendering() {
    let mut doc = Doc::new();
    let id = insert_text_path(&mut doc, straight_text_path("Export"));
    let bounds = crate::visual_world_bounds(&doc.scene, id, 0.0).expect("exact export bounds");
    let viewport = Viewport {
        center: [
            (bounds.min_x + bounds.max_x) * 0.5,
            (bounds.min_y + bounds.max_y) * 0.5,
        ],
        zoom: 1.0,
    };

    let mut all_roots = RasterRenderer::new(220, 100).unwrap();
    all_roots.render(&doc.scene, &viewport);
    let all_pixels = all_roots.copy_rgba();

    let mut selected = RasterRenderer::new(220, 100).unwrap();
    selected.render_page(&doc.scene, &viewport, Some(id));
    let selected_pixels = selected.copy_rgba();
    assert_eq!(selected_pixels, all_pixels);
    assert!(opaque_pixel_count(&selected_pixels) > 0);

    let png = selected.encode_png().expect("shared raster path encodes");
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
}

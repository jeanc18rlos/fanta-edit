use super::*;
use crate::select::{CropTool, RegionSelectTool, RegionSelectionKind};

fn context<'a>(doc: &'a mut Doc, viewport: &'a mut Viewport) -> ToolContext<'a> {
    let mut context = ToolContext::new(
        doc,
        viewport,
        SnapEngine::default(),
        DVec2::new(800.0, 600.0),
    );
    context.draw_content_only = true;
    context
}

#[test]
fn draw_rectangle_selects_vector_child_instead_of_design_frame() {
    let mut doc = Doc::new();
    let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([200.0, 200.0]),
        ..GroupNode::default()
    }));
    frame.transform = Transform2D::translation(-100.0, -100.0);
    let frame_id = frame.id;
    doc.apply(Operation::create_node(frame))
        .expect("create frame");
    let mut child = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        50.0,
        50.0,
        40.0,
        40.0,
        Color::WHITE,
    )));
    child.parent = Some(frame_id);
    let child_id = child.id;
    doc.apply(Operation::create_node(child))
        .expect("create vector");
    let mut viewport = Viewport::default();
    let mut context = context(&mut doc, &mut viewport);
    let mut tool = RectangleSelectTool::new();

    tool.handle_event(
        &mut context,
        pe_press([400.0, 300.0], ModifierKeys::empty()),
    );
    tool.handle_event(
        &mut context,
        pe_release([400.0, 300.0], ModifierKeys::empty()),
    );
    assert!(context.doc.selection.is_empty());

    tool.handle_event(
        &mut context,
        pe_press([365.0, 265.0], ModifierKeys::empty()),
    );
    tool.handle_event(
        &mut context,
        pe_release([365.0, 265.0], ModifierKeys::empty()),
    );
    assert_eq!(context.doc.selection.as_slice(), &[child_id]);
    assert!(!context.doc.selection.contains(frame_id));
}

#[test]
fn ellipse_and_polygonal_lasso_select_vector_regions() {
    let mut doc = Doc::new();
    let center = rect_at(&mut doc, -10.0, -10.0, 20.0, 20.0);
    let corner = rect_at(&mut doc, 65.0, 65.0, 20.0, 20.0);
    let mut viewport = Viewport::default();
    let mut context = context(&mut doc, &mut viewport);
    let mut ellipse = RegionSelectTool::new(RegionSelectionKind::Ellipse);

    ellipse.handle_event(
        &mut context,
        pe_press([350.0, 250.0], ModifierKeys::empty()),
    );
    ellipse.handle_event(&mut context, pe_move([450.0, 350.0], ModifierKeys::empty()));
    ellipse.handle_event(
        &mut context,
        pe_release([450.0, 350.0], ModifierKeys::empty()),
    );
    assert_eq!(context.doc.selection.as_slice(), &[center]);

    let mut polygon = RegionSelectTool::new(RegionSelectionKind::PolygonalLasso);
    for point in [
        [450.0, 350.0],
        [500.0, 350.0],
        [500.0, 400.0],
        [450.0, 400.0],
    ] {
        polygon.handle_event(&mut context, pe_press(point, ModifierKeys::empty()));
        polygon.handle_event(&mut context, pe_release(point, ModifierKeys::empty()));
    }
    polygon.handle_event(
        &mut context,
        ToolEvent::Key(KeyEvent::press(LogicalKey::Enter)),
    );
    assert_eq!(context.doc.selection.as_slice(), &[corner]);
}

#[test]
fn freehand_lasso_selects_content_inside_the_drawn_outline() {
    let mut doc = Doc::new();
    let inside = rect_at(&mut doc, -10.0, -10.0, 20.0, 20.0);
    let outside = rect_at(&mut doc, 100.0, 100.0, 20.0, 20.0);
    let mut viewport = Viewport::default();
    let mut context = context(&mut doc, &mut viewport);
    let mut lasso = RegionSelectTool::new(RegionSelectionKind::Lasso);

    lasso.handle_event(
        &mut context,
        pe_press([360.0, 260.0], ModifierKeys::empty()),
    );
    for point in [[440.0, 260.0], [440.0, 340.0], [360.0, 340.0]] {
        lasso.handle_event(&mut context, pe_move(point, ModifierKeys::empty()));
    }
    lasso.handle_event(
        &mut context,
        pe_release([360.0, 260.0], ModifierKeys::empty()),
    );
    assert_eq!(context.doc.selection.as_slice(), &[inside]);
    assert!(!context.doc.selection.contains(outside));
}

#[test]
fn magic_wand_uses_tolerance_and_contiguity_on_vector_paints() {
    let mut doc = Doc::new();
    let mut make = |x, color| {
        let node = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            x, -10.0, 20.0, 20.0, color,
        )));
        let id = node.id;
        doc.apply(Operation::create_node(node))
            .expect("create vector");
        id
    };
    let seed = make(-100.0, Color::rgb(200, 20, 20));
    let near = make(-80.0, Color::rgb(210, 20, 20));
    let far = make(100.0, Color::rgb(210, 20, 20));
    let mut viewport = Viewport::default();
    let mut context = context(&mut doc, &mut viewport);
    let mut wand = RegionSelectTool::new(RegionSelectionKind::MagicWand);

    context.selection_tolerance = 15;
    context.selection_contiguous = true;
    wand.handle_event(
        &mut context,
        pe_press([310.0, 300.0], ModifierKeys::empty()),
    );
    assert_eq!(context.doc.selection.len(), 2);
    assert!(context.doc.selection.contains(seed));
    assert!(context.doc.selection.contains(near));
    assert!(!context.doc.selection.contains(far));

    context.selection_contiguous = false;
    wand.handle_event(
        &mut context,
        pe_press([310.0, 300.0], ModifierKeys::empty()),
    );
    assert_eq!(context.doc.selection.len(), 3);
    assert!(context.doc.selection.contains(seed));
    assert!(context.doc.selection.contains(near));
    assert!(context.doc.selection.contains(far));

    context.selection_tolerance = 0;
    wand.handle_event(
        &mut context,
        pe_press([310.0, 300.0], ModifierKeys::empty()),
    );
    assert_eq!(context.doc.selection.as_slice(), &[seed]);
}

#[test]
fn crop_applies_vector_clip_and_undo_restores_original() {
    let mut doc = Doc::new();
    let id = rect_at(&mut doc, -100.0, -100.0, 200.0, 200.0);
    doc.selection.replace_with([id]);
    let undo_before = doc.history.undo_depth();
    let mut viewport = Viewport::default();
    let mut context = context(&mut doc, &mut viewport);
    let mut crop = CropTool::new();

    crop.handle_event(
        &mut context,
        pe_press([350.0, 250.0], ModifierKeys::empty()),
    );
    let response = crop.handle_event(
        &mut context,
        pe_release([450.0, 350.0], ModifierKeys::empty()),
    );
    assert!(
        response
            .overlays
            .iter()
            .any(|overlay| matches!(overlay, ToolOverlay::PreviewRect { .. }))
    );
    assert_eq!(context.doc.history.undo_depth(), undo_before);
    crop.handle_event(
        &mut context,
        ToolEvent::Key(KeyEvent::press(LogicalKey::Enter)),
    );
    assert_eq!(context.doc.history.undo_depth(), undo_before + 1);
    let group_id = context.doc.selection.as_slice()[0];
    let group = context.doc.scene.get(group_id).expect("crop group");
    assert!(
        matches!(&group.data, NodeData::Group(group) if group.clip_size == Some([100.0, 100.0]))
    );
    assert_eq!(
        context.doc.scene.get(id).expect("vector").parent,
        Some(group_id)
    );
    assert_eq!(
        context
            .doc
            .scene
            .world_bounds(group_id)
            .expect("clip bounds"),
        Bounds::from_xywh(-50.0, -50.0, 100.0, 100.0)
    );
    context.doc.undo().expect("undo crop");
    assert_eq!(
        context.doc.scene.get(id).expect("restored vector").parent,
        None
    );
}

#[test]
fn crop_applies_to_bitmap_and_cancel_preserves_it() {
    let mut doc = Doc::new();
    let bitmap = CanvasNode::new(NodeData::Bitmap(fanta_doc::BitmapNode {
        asset: fanta_doc::AssetId::new(),
        natural_size: [200, 100],
        local_size: [200.0, 100.0],
        crop: None,
        fit: fanta_doc::ImageFitMode::Fill,
        tint: None,
    }));
    let id = bitmap.id;
    doc.apply(Operation::create_node(bitmap))
        .expect("create image");
    doc.selection.replace_with([id]);
    let mut viewport = Viewport::default();
    let mut context = context(&mut doc, &mut viewport);
    let mut crop = CropTool::new();
    crop.handle_event(
        &mut context,
        pe_press([400.0, 300.0], ModifierKeys::empty()),
    );
    crop.handle_event(
        &mut context,
        pe_release([500.0, 350.0], ModifierKeys::empty()),
    );
    crop.handle_event(
        &mut context,
        ToolEvent::Key(KeyEvent::press(LogicalKey::Escape)),
    );
    assert_eq!(context.doc.scene.get(id).expect("image").parent, None);

    context.crop_aspect_ratio = Some(1.0);
    crop.handle_event(
        &mut context,
        pe_press([400.0, 300.0], ModifierKeys::empty()),
    );
    crop.handle_event(
        &mut context,
        pe_release([500.0, 350.0], ModifierKeys::empty()),
    );
    crop.handle_event(
        &mut context,
        ToolEvent::Key(KeyEvent::press(LogicalKey::Enter)),
    );
    let group_id = context.doc.selection.as_slice()[0];
    let group = context.doc.scene.get(group_id).expect("crop group");
    assert!(
        matches!(&group.data, NodeData::Group(group) if group.clip_size == Some([100.0, 100.0]))
    );
    assert_eq!(
        context.doc.scene.get(id).expect("image").parent,
        Some(group_id)
    );
}

#[test]
fn small_draw_rectangle_inside_bitmap_can_be_applied_as_crop_and_undone() {
    let mut doc = Doc::new();
    let mut bitmap = CanvasNode::new(NodeData::Bitmap(fanta_doc::BitmapNode {
        asset: fanta_doc::AssetId::new(),
        natural_size: [200, 200],
        local_size: [200.0, 200.0],
        crop: None,
        fit: fanta_doc::ImageFitMode::Stretch,
        tint: None,
    }));
    bitmap.transform = Transform2D::translation(-100.0, -100.0);
    let bitmap_id = bitmap.id;
    doc.apply(Operation::create_node(bitmap))
        .expect("create bitmap");
    let mut viewport = Viewport::default();
    let mut context = context(&mut doc, &mut viewport);
    let mut rectangle = RectangleSelectTool::new();
    rectangle.handle_event(
        &mut context,
        pe_press([380.0, 280.0], ModifierKeys::empty()),
    );
    rectangle.handle_event(
        &mut context,
        pe_release([420.0, 320.0], ModifierKeys::empty()),
    );
    assert_eq!(context.doc.selection.as_slice(), &[bitmap_id]);
    assert!(matches!(
        context.draw_selection_region.as_ref().map(|region| &region.shape),
        Some(crate::select::DrawSelectionShape::Rectangle(bounds))
            if *bounds == Bounds::from_xywh(-20.0, -20.0, 40.0, 40.0)
    ));

    let mut crop = CropTool::new();
    crop.handle_event(
        &mut context,
        ToolEvent::Key(KeyEvent::press(LogicalKey::Enter)),
    );
    let crop_id = context.doc.selection.as_slice()[0];
    let crop_group = context.doc.scene.get(crop_id).expect("crop group");
    assert!(
        matches!(&crop_group.data, NodeData::Group(group) if group.clip_size == Some([40.0, 40.0]))
    );
    assert_eq!(
        context.doc.scene.get(bitmap_id).expect("bitmap").parent,
        Some(crop_id)
    );
    context.doc.undo().expect("undo crop");
    assert_eq!(
        context
            .doc
            .scene
            .get(bitmap_id)
            .expect("restored bitmap")
            .parent,
        None
    );
}

#[test]
fn small_draw_ellipse_and_lasso_inside_bitmap_select_it() {
    let mut doc = Doc::new();
    let mut bitmap = CanvasNode::new(NodeData::Bitmap(fanta_doc::BitmapNode {
        asset: fanta_doc::AssetId::new(),
        natural_size: [200, 200],
        local_size: [200.0, 200.0],
        crop: None,
        fit: fanta_doc::ImageFitMode::Stretch,
        tint: None,
    }));
    bitmap.transform = Transform2D::translation(-100.0, -100.0);
    let bitmap_id = bitmap.id;
    doc.apply(Operation::create_node(bitmap))
        .expect("create bitmap");
    let mut viewport = Viewport::default();
    let mut context = context(&mut doc, &mut viewport);

    let mut ellipse = RegionSelectTool::new(RegionSelectionKind::Ellipse);
    ellipse.handle_event(
        &mut context,
        pe_press([380.0, 280.0], ModifierKeys::empty()),
    );
    ellipse.handle_event(
        &mut context,
        pe_release([420.0, 320.0], ModifierKeys::empty()),
    );
    assert_eq!(context.doc.selection.as_slice(), &[bitmap_id]);

    let mut lasso = RegionSelectTool::new(RegionSelectionKind::Lasso);
    lasso.handle_event(
        &mut context,
        pe_press([380.0, 280.0], ModifierKeys::empty()),
    );
    for point in [[420.0, 280.0], [420.0, 320.0], [380.0, 320.0]] {
        lasso.handle_event(&mut context, pe_move(point, ModifierKeys::empty()));
    }
    lasso.handle_event(
        &mut context,
        pe_release([380.0, 280.0], ModifierKeys::empty()),
    );
    assert_eq!(context.doc.selection.as_slice(), &[bitmap_id]);
}

#[test]
fn ellipse_selection_crops_vector_with_an_undoable_shape_mask() {
    let mut doc = Doc::new();
    let id = rect_at(&mut doc, -100.0, -100.0, 200.0, 200.0);
    let mut viewport = Viewport::default();
    let mut context = context(&mut doc, &mut viewport);
    let mut ellipse = RegionSelectTool::new(RegionSelectionKind::Ellipse);
    ellipse.handle_event(&mut context, pe_press([350.0, 250.0], ModifierKeys::ALT));
    ellipse.handle_event(&mut context, pe_release([450.0, 350.0], ModifierKeys::ALT));
    assert!(context.doc.selection.contains(id));
    assert!(matches!(
        context
            .draw_selection_region
            .as_ref()
            .map(|region| &region.shape),
        Some(crate::select::DrawSelectionShape::Ellipse(_))
    ));

    let mut crop = CropTool::new();
    crop.handle_event(
        &mut context,
        ToolEvent::Key(KeyEvent::press(LogicalKey::Enter)),
    );
    let group_id = context.doc.selection.as_slice()[0];
    let children = context.doc.scene.children_of(Some(group_id));
    assert_eq!(children.len(), 2);
    assert!(context.doc.scene.get(children[0]).expect("mask").is_mask);
    assert_eq!(children[1], id);
    let mask = context.doc.scene.get(children[0]).expect("mask");
    assert!(matches!(&mask.data, NodeData::Vector(vector) if vector.path.segments.len() == 6));
    assert!(context.draw_selection_region.is_none());
    context.doc.undo().expect("undo crop");
    assert_eq!(context.doc.scene.get(id).expect("vector").parent, None);
}

#[test]
fn subtract_selection_builds_a_compound_crop_mask() {
    let mut doc = Doc::new();
    let id = rect_at(&mut doc, -100.0, -100.0, 200.0, 200.0);
    let mut viewport = Viewport::default();
    let mut context = context(&mut doc, &mut viewport);
    let mut rectangle = RectangleSelectTool::new();
    rectangle.handle_event(&mut context, pe_press([300.0, 200.0], ModifierKeys::ALT));
    rectangle.handle_event(&mut context, pe_release([500.0, 400.0], ModifierKeys::ALT));
    assert!(context.doc.selection.contains(id));

    context.rectangle_selection_operation = RectangleSelectionOperation::Subtract;
    let mut ellipse = RegionSelectTool::new(RegionSelectionKind::Ellipse);
    ellipse.handle_event(&mut context, pe_press([350.0, 250.0], ModifierKeys::ALT));
    ellipse.handle_event(&mut context, pe_release([450.0, 350.0], ModifierKeys::ALT));
    assert!(context.doc.selection.contains(id));
    assert!(matches!(
        context
            .draw_selection_region
            .as_ref()
            .map(|region| &region.shape),
        Some(crate::select::DrawSelectionShape::Combined {
            operation: crate::select::DrawShapeOperation::Subtract,
            ..
        })
    ));

    let mut crop = CropTool::new();
    crop.handle_event(
        &mut context,
        ToolEvent::Key(KeyEvent::press(LogicalKey::Enter)),
    );
    let crop_id = context.doc.selection.as_slice()[0];
    let mask_id = context.doc.scene.children_of(Some(crop_id))[0];
    let mask = context.doc.scene.get(mask_id).expect("compound mask");
    assert!(
        matches!(&mask.data, NodeData::Boolean(boolean) if boolean.op == fanta_doc::BooleanOp::Subtract)
    );
    assert!(mask.is_mask);
    assert_eq!(context.doc.scene.children_of(Some(mask_id)).len(), 2);
}

#[test]
fn bitmap_wand_pixel_region_crops_with_a_disconnected_mask_and_undo() {
    let mut doc = Doc::new();
    let bitmap = CanvasNode::new(NodeData::Bitmap(fanta_doc::BitmapNode {
        asset: fanta_doc::AssetId::new(),
        natural_size: [4, 4],
        local_size: [4.0, 4.0],
        crop: None,
        fit: fanta_doc::ImageFitMode::Stretch,
        tint: None,
    }));
    let bitmap_id = bitmap.id;
    doc.apply(Operation::create_node(bitmap))
        .expect("create bitmap");
    let mut viewport = Viewport::default();
    let mut context = context(&mut doc, &mut viewport);
    context.bitmap_wand_override = Some((
        bitmap_id,
        crate::select::DrawSelectionShape::RasterRuns {
            quads: vec![
                [[0.0, 0.0], [1.0, 0.0], [1.0, 4.0], [0.0, 4.0]],
                [[3.0, 0.0], [4.0, 0.0], [4.0, 4.0], [3.0, 4.0]],
            ],
            bounds: Bounds::from_xywh(0.0, 0.0, 4.0, 4.0),
        },
    ));
    let mut wand = RegionSelectTool::new(RegionSelectionKind::MagicWand);
    wand.handle_event(
        &mut context,
        pe_press([400.5, 300.5], ModifierKeys::empty()),
    );
    assert_eq!(context.doc.selection.as_slice(), &[bitmap_id]);
    assert!(matches!(
        context.draw_selection_region.as_ref().map(|region| &region.shape),
        Some(crate::select::DrawSelectionShape::RasterRuns { quads, .. }) if quads.len() == 2
    ));

    let mut crop = CropTool::new();
    crop.handle_event(
        &mut context,
        ToolEvent::Key(KeyEvent::press(LogicalKey::Enter)),
    );
    let crop_id = context.doc.selection.as_slice()[0];
    let children = context.doc.scene.children_of(Some(crop_id));
    assert_eq!(children.len(), 2);
    let mask = context.doc.scene.get(children[0]).expect("pixel mask");
    assert!(mask.is_mask);
    assert!(matches!(&mask.data, NodeData::Vector(vector) if vector.path.segments.len() == 10));
    assert_eq!(children[1], bitmap_id);
    context.doc.undo().expect("undo pixel crop");
    assert_eq!(
        context.doc.scene.get(bitmap_id).expect("bitmap").parent,
        None
    );
}

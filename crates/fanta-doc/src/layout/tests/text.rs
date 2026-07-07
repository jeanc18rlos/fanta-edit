//! Auto-width text: WidthAndHeight labels snap to the injected glyph
//! measure; Height/None modes keep authored width (measure never fires).

use super::*;

// ---------------------------------------------------------------------------
// Auto-width text via injected measure
// ---------------------------------------------------------------------------

#[test]
fn auto_width_text_snaps_to_measured_glyph_width() {
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Horizontal,
        spacing: 4.0,
        ..Default::default()
    };
    let f = t.push(frame(400.0, 50.0, al));

    // Two auto-width labels: "Edit" (4 chars) and "Copy" (4 chars). The authored
    // box width (999) is wrong/too wide; measure overwrites it to chars*7 = 28.
    let mut edit = TextNode::new("Edit", 999.0, 20.0);
    edit.auto_resize = TextAutoResize::WidthAndHeight;
    let mut edit_node = CanvasNode::new(NodeData::Text(edit));
    edit_node.parent = Some(f);
    let edit_id = t.push(edit_node);

    let mut copy = TextNode::new("Copy", 999.0, 20.0);
    copy.auto_resize = TextAutoResize::WidthAndHeight;
    let mut copy_node = CanvasNode::new(NodeData::Text(copy));
    copy_node.parent = Some(f);
    let copy_id = t.push(copy_node);

    let mut measure = fake_measure(7.0, 16.0);
    solve_auto_layout(&mut t, f, &mut measure);

    // Each label measured to 4*7 = 28 wide, 16 tall (never wrapped).
    approx(placed_size(&t, edit_id), [28.0, 16.0]);
    approx(placed_size(&t, copy_id), [28.0, 16.0]);
    // Packed: edit@0, copy@28+4 = 32.
    approx(placed_origin(&t, edit_id), [0.0, 0.0]);
    approx(placed_origin(&t, copy_id), [32.0, 0.0]);
}

#[test]
fn height_and_none_mode_text_keep_authored_width() {
    let mut t = VecTree::new();
    let f = t.push(frame(
        400.0,
        50.0,
        AutoLayout {
            mode: LayoutMode::Horizontal,
            ..Default::default()
        },
    ));

    // Height-mode keeps width; None-mode keeps both. Neither calls measure for
    // width — but apply_text_autoresize only fires for WidthAndHeight, so a
    // panicking measurer proves neither is touched.
    let mut h = TextNode::new("Wrapped", 120.0, 20.0);
    h.auto_resize = TextAutoResize::Height;
    let mut hn = CanvasNode::new(NodeData::Text(h));
    hn.parent = Some(f);
    let hid = t.push(hn);

    let mut none = TextNode::new("Fixed", 80.0, 30.0);
    none.auto_resize = TextAutoResize::None;
    let mut nn = CanvasNode::new(NodeData::Text(none));
    nn.parent = Some(f);
    let nid = t.push(nn);

    solve_auto_layout(&mut t, f, &mut no_measure);

    approx(placed_size(&t, hid), [120.0, 20.0]);
    approx(placed_size(&t, nid), [80.0, 30.0]);
}

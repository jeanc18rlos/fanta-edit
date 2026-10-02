//! Auto-width text: WidthAndHeight labels snap to the injected glyph
//! measure; Height wraps at its authored width and None keeps authored geometry.

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

    solve_auto_layout(&mut t, f, &mut |text| {
        assert_eq!(text.content, "Wrapped");
        assert_eq!(text.local_size[0], 120.0);
        (100.0, 48.0)
    });

    approx(placed_size(&t, hid), [120.0, 48.0]);
    approx(placed_size(&t, nid), [80.0, 30.0]);
}

#[test]
fn stretched_card_remeasures_wrapped_text_and_hugs_height_in_one_pass() {
    let mut tree = VecTree::new();
    let outer = tree.push(frame(
        240.0,
        600.0,
        AutoLayout {
            mode: LayoutMode::Vertical,
            counter_align: CounterAlign::Stretch,
            ..Default::default()
        },
    ));
    let mut card = frame(
        400.0,
        1.0,
        AutoLayout {
            mode: LayoutMode::Vertical,
            primary_sizing: AxisSizing::Hug,
            counter_align: CounterAlign::Stretch,
            padding: [16.0; 4],
            spacing: 12.0,
            ..Default::default()
        },
    );
    card.parent = Some(outer);
    let card = tree.push(card);
    let mut paragraph = TextNode::new("A wrapped paragraph", 368.0, 1.0);
    paragraph.auto_resize = TextAutoResize::Height;
    let mut paragraph = CanvasNode::new(NodeData::Text(paragraph));
    paragraph.parent = Some(card);
    let paragraph = tree.push(paragraph);
    let footer = tree.push(rect_child(card, 60.0, 24.0));
    let mut measure = |text: &TextNode| {
        (
            text.local_size[0],
            (600.0 / text.local_size[0]).ceil() * 20.0,
        )
    };

    solve_auto_layout(&mut tree, outer, &mut measure);

    approx(placed_size(&tree, paragraph), [208.0, 60.0]);
    approx(placed_origin(&tree, footer), [16.0, 88.0]);
    approx(placed_size(&tree, card), [240.0, 128.0]);
    solve_auto_layout(&mut tree, outer, &mut measure);
    approx(placed_size(&tree, card), [240.0, 128.0]);
    approx(placed_origin(&tree, footer), [16.0, 88.0]);
}

#[test]
fn typography_and_content_modes_reflow_cards_without_rewriting_literals() {
    use crate::{
        BoundProp, Doc, Mode, ModeId, Operation, VarValue, Variable, VariableCollection,
        VariableCollectionId, VariableId, VariableType,
    };
    use std::collections::BTreeMap;
    let mut doc = Doc::new();
    let card = frame(
        200.0,
        1.0,
        AutoLayout {
            mode: LayoutMode::Vertical,
            primary_sizing: AxisSizing::Hug,
            counter_align: CounterAlign::Stretch,
            padding: [16.0; 4],
            ..Default::default()
        },
    );
    let card_id = card.id;
    doc.apply(Operation::create_node(card)).expect("card");
    let collection = VariableCollectionId::new();
    let light = ModeId::new();
    let dark = ModeId::new();
    doc.variables.collections.insert(
        collection,
        VariableCollection {
            id: collection,
            name: "Theme".into(),
            modes: vec![
                Mode {
                    id: light,
                    name: "Light".into(),
                },
                Mode {
                    id: dark,
                    name: "Dark".into(),
                },
            ],
            default_mode: light,
            variable_order: Vec::new(),
        },
    );
    let typography = VariableId::new();
    let content = VariableId::new();
    let mut small = crate::TextStyle::default();
    small.size_px = 16.0;
    small.line_height = 1.5;
    let mut large = small.clone();
    large.size_px = 32.0;
    doc.variables.variables.insert(
        typography,
        Variable {
            id: typography,
            collection,
            name: "Body".into(),
            ty: VariableType::Typography,
            values_by_mode: BTreeMap::from([
                (light, VarValue::TextStyle { value: small }),
                (dark, VarValue::TextStyle { value: large }),
            ]),
            scopes: Vec::new(),
        },
    );
    doc.variables.variables.insert(
        content,
        Variable {
            id: content,
            collection,
            name: "Copy".into(),
            ty: VariableType::String,
            values_by_mode: BTreeMap::from([
                (
                    light,
                    VarValue::String {
                        value: "Brief".into(),
                    },
                ),
                (
                    dark,
                    VarValue::String {
                        value: "A much longer paragraph for the narrow card".into(),
                    },
                ),
            ]),
            scopes: Vec::new(),
        },
    );
    let mut paragraph = TextNode::new("Authored fallback", 400.0, 1.0);
    paragraph.auto_resize = TextAutoResize::Height;
    let literal_style = paragraph.style.clone();
    let mut paragraph = CanvasNode::new(NodeData::Text(paragraph));
    paragraph.parent = Some(card_id);
    paragraph.bindings.insert(BoundProp::TextStyle, typography);
    paragraph.bindings.insert(BoundProp::TextContent, content);
    let paragraph_id = paragraph.id;
    doc.apply(Operation::create_node(paragraph))
        .expect("paragraph");
    let mut measure = |text: &TextNode| {
        let width = text.content.chars().count() as f64 * text.style.size_px * 0.5;
        (
            text.local_size[0],
            (width / text.local_size[0]).ceil() * text.style.size_px * text.style.line_height,
        )
    };
    let mut heights = Vec::new();
    for mode in [light, dark, light] {
        doc.active_modes.insert(collection, mode);
        solve_auto_layout_with_variables(
            &mut doc.scene,
            card_id,
            &doc.variables,
            &doc.active_modes,
            &mut measure,
        );
        let paragraph = doc.scene.get(paragraph_id).expect("paragraph");
        let NodeData::Text(text) = &paragraph.data else {
            panic!("text");
        };
        assert_eq!(text.content, "Authored fallback");
        assert_eq!(text.style, literal_style);
        assert_eq!(paragraph.bindings.len(), 2);
        let NodeData::Group(card) = &doc.scene.get(card_id).expect("card").data else {
            panic!("card");
        };
        assert_eq!(card.clip_size, Some([200.0, text.local_size[1] + 32.0]));
        heights.push(text.local_size[1]);
    }
    assert_eq!(heights[0], 24.0);
    assert!(heights[1] > heights[0]);
    assert_eq!(heights[2], heights[0]);
}

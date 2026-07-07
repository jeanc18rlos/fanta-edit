//! Import-fidelity gaps A–D, masks, and auto-layout import.

use super::*;

// =============================================================================
// Import-fidelity gaps A–D (stroke-only outlines, font weights, stroke
// caps/joins/dash, textCase)
// =============================================================================

fn first_text(doc: &Doc) -> fanta_doc::node::TextNode {
    for root in doc.scene.roots().to_vec() {
        for id in doc.scene.descendants_of(root) {
            if let Some(NodeData::Text(t)) = doc.scene.get(id).map(|n| &n.data) {
                return t.clone();
            }
        }
    }
    panic!("no text node in scene");
}

#[test]
fn gap_a_stroke_only_vector_paints_outline_as_fill_without_width_stroke() {
    // A Lucide/Feather-style stroke-only icon: NO visible fill paint, a visible
    // stroke paint, and a `fillGeometry` that is the already-expanded outline.
    // We must paint that outline AS a fill (the stroke's blue paint) and emit NO
    // width stroke — otherwise the glyph double-strokes (blobby).
    let mut outline = Vec::new();
    outline.extend(cmd(1, &[2.0, 2.0])); // Move
    outline.extend(cmd(2, &[20.0, 2.0])); // Line
    outline.extend(cmd(2, &[11.0, 16.0])); // Line
    outline.extend(cmd(0, &[])); // Close

    let fig = doc_from_with_blobs(
        vec![o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("VECTOR".to_owned())),
                ("name", KiwiValue::String("icon".to_owned())),
                ("size", vector(24.0, 18.0)),
                // No fillPaints at all → no visible fill.
                (
                    "strokePaints",
                    KiwiValue::Array(vec![solid_paint(0.0, 0.0, 1.0, 1.0)]),
                ),
                ("strokeWeight", KiwiValue::Float(2.0)),
                (
                    "fillGeometry",
                    KiwiValue::Array(vec![fig_path(0, "NONZERO")]),
                ),
            ],
        )],
        vec![outline],
    );

    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(
        report.stroke_only_vectors, 1,
        "detected as stroke-only outline"
    );
    assert_eq!(report.geometry_decoded, 1, "outline decoded into the path");

    let node = doc.scene.get(doc.scene.roots()[0]).unwrap();
    assert_eq!(node.meta["stroke_only_outline"], true);
    match &node.data {
        NodeData::Vector(v) => {
            // The stroke paint became the fill…
            assert_eq!(
                v.fills.first().cloned(),
                Some(Fill::solid(Color::rgba(0, 0, 255, 255))),
                "stroke paint renders as the outline fill"
            );
            // …and there is NO width stroke (no double-stroke).
            assert!(v.strokes.is_empty(), "no width stroke added on top");
        }
        other => panic!("expected vector, got {other:?}"),
    }
}

#[test]
fn gap_a_vector_with_visible_fill_keeps_normal_fill_plus_stroke() {
    // A vector that HAS a visible fill is NOT stroke-only: keep the fill and add
    // the width stroke as before.
    let mut tri = Vec::new();
    tri.extend(cmd(1, &[2.0, 2.0]));
    tri.extend(cmd(2, &[20.0, 2.0]));
    tri.extend(cmd(2, &[11.0, 16.0]));
    tri.extend(cmd(0, &[]));

    let fig = doc_from_with_blobs(
        vec![o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("VECTOR".to_owned())),
                ("size", vector(24.0, 18.0)),
                (
                    "fillPaints",
                    KiwiValue::Array(vec![solid_paint(1.0, 0.0, 0.0, 1.0)]),
                ),
                (
                    "strokePaints",
                    KiwiValue::Array(vec![solid_paint(0.0, 0.0, 1.0, 1.0)]),
                ),
                ("strokeWeight", KiwiValue::Float(2.0)),
                (
                    "fillGeometry",
                    KiwiValue::Array(vec![fig_path(0, "NONZERO")]),
                ),
            ],
        )],
        vec![tri],
    );

    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(
        report.stroke_only_vectors, 0,
        "has a visible fill → not stroke-only"
    );
    let v = first_vector(&doc);
    assert_eq!(
        v.fills.first().cloned(),
        Some(Fill::solid(Color::rgba(255, 0, 0, 255)))
    );
    assert_eq!(v.strokes.len(), 1, "width stroke kept");
}

#[test]
fn gap_a_invisible_stroke_paint_is_not_stroke_only() {
    // A stroke paint marked `visible: false` does not count as a visible stroke;
    // such a node is neither fill nor stroke and must not be treated as a
    // stroke-only outline.
    let mut tri = Vec::new();
    tri.extend(cmd(1, &[0.0, 0.0]));
    tri.extend(cmd(2, &[10.0, 0.0]));
    tri.extend(cmd(2, &[5.0, 8.0]));
    tri.extend(cmd(0, &[]));
    let hidden_stroke = o(
        "Paint",
        vec![
            ("type", KiwiValue::Enum("SOLID".to_owned())),
            ("color", color(0.0, 0.0, 1.0, 1.0)),
            ("visible", KiwiValue::Bool(false)),
        ],
    );
    let fig = doc_from_with_blobs(
        vec![o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("VECTOR".to_owned())),
                ("size", vector(12.0, 10.0)),
                ("strokePaints", KiwiValue::Array(vec![hidden_stroke])),
                ("strokeWeight", KiwiValue::Float(2.0)),
                (
                    "fillGeometry",
                    KiwiValue::Array(vec![fig_path(0, "NONZERO")]),
                ),
            ],
        )],
        vec![tri],
    );
    let (_doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(
        report.stroke_only_vectors, 0,
        "hidden stroke is not a visible stroke"
    );
}

#[test]
fn gap_b_font_weights_parse_full_opentype_table() {
    // Each style string maps to its OpenType numeric weight; "Italic" sets italic.
    let cases: &[(&str, u16, bool)] = &[
        ("Thin", 100, false),
        ("ExtraLight", 200, false),
        ("Light", 300, false),
        ("Regular", 400, false),
        ("Medium", 500, false),
        ("SemiBold", 600, false),
        ("Bold", 700, false),
        ("ExtraBold", 800, false),
        ("Black", 900, false),
        ("Light Italic", 300, true),
        ("SemiBold Italic", 600, true),
    ];
    for (style, weight, italic) in cases {
        let fig = doc_from(vec![o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("TEXT".to_owned())),
                ("size", vector(100.0, 20.0)),
                ("textData", text_data("x")),
                ("fontSize", KiwiValue::Float(16.0)),
                ("fontName", font_name("Inter", style)),
            ],
        )]);
        let (doc, _, _) = fig_to_doc(&fig).unwrap();
        let t = first_text(&doc);
        assert_eq!(t.style.weight, *weight, "weight for style {style:?}");
        assert_eq!(t.style.italic, *italic, "italic for style {style:?}");
    }
}

#[test]
fn gap_b_report_counts_non_default_weights() {
    // SemiBold (600) is a non-default weight; 400/700 are not counted.
    let semibold = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(0, 1)),
            ("type", KiwiValue::Enum("TEXT".to_owned())),
            ("size", vector(100.0, 20.0)),
            ("textData", text_data("x")),
            ("fontName", font_name("Inter", "SemiBold")),
        ],
    )]);
    let (_doc, report, _) = fig_to_doc(&semibold).unwrap();
    assert_eq!(report.non_default_weights, 1);

    let regular = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(0, 1)),
            ("type", KiwiValue::Enum("TEXT".to_owned())),
            ("size", vector(100.0, 20.0)),
            ("textData", text_data("x")),
            ("fontName", font_name("Inter", "Regular")),
        ],
    )]);
    let (_doc, report, _) = fig_to_doc(&regular).unwrap();
    assert_eq!(report.non_default_weights, 0, "400 is a default weight");
}

#[test]
fn gap_c_stroke_cap_join_and_dash_are_imported() {
    let rect = o(
        "NodeChange",
        vec![
            ("guid", guid(0, 2)),
            ("parentIndex", parent_index(0, 1)),
            ("type", KiwiValue::Enum("RECTANGLE".to_owned())),
            ("size", vector(100.0, 50.0)),
            (
                "strokePaints",
                KiwiValue::Array(vec![solid_paint(0.0, 0.0, 0.0, 1.0)]),
            ),
            ("strokeWeight", KiwiValue::Float(2.0)),
            ("strokeCap", KiwiValue::Enum("ROUND".to_owned())),
            ("strokeJoin", KiwiValue::Enum("BEVEL".to_owned())),
            (
                "dashPattern",
                KiwiValue::Array(vec![KiwiValue::Float(4.0), KiwiValue::Float(3.0)]),
            ),
        ],
    );
    let (doc, report, _) = fig_to_doc(&doc_with_shape(rect)).unwrap();
    assert_eq!(report.strokes_with_cap_join_dash, 1);
    let v = first_vector(&doc);
    let s = &v.strokes[0];
    assert_eq!(s.cap, StrokeCap::Round);
    assert_eq!(s.join, StrokeJoin::Bevel);
    assert_eq!(s.dash, vec![4.0, 3.0]);
}

#[test]
fn gap_c_default_cap_join_no_dash_not_counted() {
    // A plain stroke (no cap/join/dash) keeps doc defaults and is NOT counted as
    // a cap/join/dash stroke.
    let rect = o(
        "NodeChange",
        vec![
            ("guid", guid(0, 2)),
            ("parentIndex", parent_index(0, 1)),
            ("type", KiwiValue::Enum("RECTANGLE".to_owned())),
            ("size", vector(100.0, 50.0)),
            (
                "strokePaints",
                KiwiValue::Array(vec![solid_paint(0.0, 0.0, 0.0, 1.0)]),
            ),
            ("strokeWeight", KiwiValue::Float(2.0)),
        ],
    );
    let (doc, report, _) = fig_to_doc(&doc_with_shape(rect)).unwrap();
    assert_eq!(report.strokes_with_cap_join_dash, 0);
    let v = first_vector(&doc);
    let s = &v.strokes[0];
    assert_eq!(s.cap, StrokeCap::Butt, "NONE/absent cap → butt default");
    assert_eq!(s.join, StrokeJoin::Miter, "absent join → miter default");
    assert!(s.dash.is_empty(), "no dash pattern");
}

#[test]
fn gap_c_stroke_miter_angle_maps_to_miter_limit() {
    // strokeMiterAngle = 60° → miter_limit = 1 / sin(30°) = 2.0.
    let rect = o(
        "NodeChange",
        vec![
            ("guid", guid(0, 2)),
            ("parentIndex", parent_index(0, 1)),
            ("type", KiwiValue::Enum("RECTANGLE".to_owned())),
            ("size", vector(100.0, 50.0)),
            (
                "strokePaints",
                KiwiValue::Array(vec![solid_paint(0.0, 0.0, 0.0, 1.0)]),
            ),
            ("strokeWeight", KiwiValue::Float(2.0)),
            ("strokeMiterAngle", KiwiValue::Float(60.0)),
        ],
    );
    let (doc, _, _) = fig_to_doc(&doc_with_shape(rect)).unwrap();
    let v = first_vector(&doc);
    assert!(
        (v.strokes[0].miter_limit - 2.0).abs() < 1e-6,
        "1/sin(30deg) = 2.0"
    );
}

#[test]
fn gap_d_text_case_upper_uppercases_content() {
    // `textCase` is a TOP-LEVEL NodeChange field (matching the real .fig schema).
    let fig = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(0, 1)),
            ("type", KiwiValue::Enum("TEXT".to_owned())),
            ("size", vector(100.0, 20.0)),
            ("textData", text_data("Submit")),
            ("textCase", KiwiValue::Enum("UPPER".to_owned())),
            ("fontName", font_name("Inter", "Regular")),
        ],
    )]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(first_text(&doc).content, "SUBMIT");
    assert_eq!(report.text_case_transformed, 1);
}

#[test]
fn gap_d_text_case_lower_title_and_original() {
    let cases: &[(&str, &str, &str, usize)] = &[
        ("HELLO World", "LOWER", "hello world", 1),
        ("hello world", "TITLE", "Hello World", 1),
        ("Keep As Is", "ORIGINAL", "Keep As Is", 0),
        // UPPER on already-uppercase text → no visible change, not counted.
        ("ABC", "UPPER", "ABC", 0),
    ];
    for (input, case, expected, count) in cases {
        let fig = doc_from(vec![o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("TEXT".to_owned())),
                ("size", vector(200.0, 20.0)),
                ("textData", text_data(input)),
                ("textCase", KiwiValue::Enum(case.to_string())),
            ],
        )]);
        let (doc, report, _) = fig_to_doc(&fig).unwrap();
        assert_eq!(
            first_text(&doc).content,
            *expected,
            "case {case} on {input:?}"
        );
        assert_eq!(
            report.text_case_transformed, *count,
            "count for {case} on {input:?}"
        );
    }
}

#[test]
fn gap_d_text_case_nested_in_text_data_is_fallback() {
    // Robustness: a schema variant placing textCase under textData still applies.
    let fig = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(0, 1)),
            ("type", KiwiValue::Enum("TEXT".to_owned())),
            ("size", vector(100.0, 20.0)),
            (
                "textData",
                o(
                    "TextData",
                    vec![
                        ("characters", KiwiValue::String("submit".to_owned())),
                        ("textCase", KiwiValue::Enum("UPPER".to_owned())),
                    ],
                ),
            ),
        ],
    )]);
    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(first_text(&doc).content, "SUBMIT");
}

/// Find the first node whose `name` matches, returning its `layout_child`.
fn layout_child_of(doc: &Doc, name: &str) -> Option<fanta_doc::node::LayoutChild> {
    for root in doc.scene.roots().to_vec() {
        for id in doc.scene.descendants_of(root) {
            let Some(n) = doc.scene.get(id) else { continue };
            if n.name == name {
                return n.layout_child;
            }
        }
    }
    panic!("no node named {name}");
}

/// Find the first node whose `name` matches, returning a clone of the whole
/// [`CanvasNode`].
fn node_named(doc: &Doc, name: &str) -> fanta_doc::node::CanvasNode {
    for root in doc.scene.roots().to_vec() {
        for id in doc.scene.descendants_of(root) {
            let Some(n) = doc.scene.get(id) else { continue };
            if n.name == name {
                return n.clone();
            }
        }
    }
    panic!("no node named {name}");
}

#[test]
fn imports_alpha_mask_flag_and_counts_it() {
    use fanta_doc::node::MaskType;
    // A RECTANGLE flagged `mask: true` with no maskType defaults to ALPHA. A
    // plain sibling carries no mask flag.
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("ROUNDED_RECTANGLE".to_owned())),
                ("name", KiwiValue::String("MaskShape".to_owned())),
                ("size", vector(40.0, 40.0)),
                ("mask", KiwiValue::Bool(true)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("type", KiwiValue::Enum("ROUNDED_RECTANGLE".to_owned())),
                ("name", KiwiValue::String("Plain".to_owned())),
                ("size", vector(40.0, 40.0)),
            ],
        ),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    let mask = node_named(&doc, "MaskShape");
    assert!(mask.is_mask, "node flagged `mask` must import as a mask");
    assert_eq!(
        mask.mask_type,
        MaskType::Alpha,
        "absent maskType defaults to ALPHA"
    );
    assert!(
        !node_named(&doc, "Plain").is_mask,
        "unflagged sibling is not a mask"
    );
    assert_eq!(report.masks_imported, 1);
    assert_eq!(report.masks_luminance, 0);
}

#[test]
fn imports_luminance_mask_type_and_counts_it() {
    use fanta_doc::node::MaskType;
    let fig = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(0, 1)),
            ("type", KiwiValue::Enum("ROUNDED_RECTANGLE".to_owned())),
            ("name", KiwiValue::String("Luma".to_owned())),
            ("size", vector(40.0, 40.0)),
            ("mask", KiwiValue::Bool(true)),
            ("maskType", KiwiValue::Enum("LUMINANCE".to_owned())),
        ],
    )]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    let mask = node_named(&doc, "Luma");
    assert!(mask.is_mask);
    assert_eq!(mask.mask_type, MaskType::Luminance);
    assert_eq!(report.masks_imported, 1);
    assert_eq!(report.masks_luminance, 1);
}

#[test]
fn vector_mask_type_collapses_to_alpha() {
    use fanta_doc::node::MaskType;
    // Figma's VECTOR / OUTLINE mask types are the alpha coverage of the shape, so
    // they import as ALPHA (the renderer's alpha path produces the outline mask).
    let fig = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(0, 1)),
            ("type", KiwiValue::Enum("ROUNDED_RECTANGLE".to_owned())),
            ("name", KiwiValue::String("Vec".to_owned())),
            ("size", vector(40.0, 40.0)),
            ("mask", KiwiValue::Bool(true)),
            ("maskType", KiwiValue::Enum("VECTOR".to_owned())),
        ],
    )]);
    let (doc, _report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(node_named(&doc, "Vec").mask_type, MaskType::Alpha);
}

#[test]
fn vector_family_node_honors_the_mask_flag() {
    use fanta_doc::node::{MaskType, NodeData};
    // A VECTOR node (which takes the dedicated `build_vector` path, not the
    // generic builder) flagged `mask` with `maskType: OUTLINE` must still import
    // as a mask — this is the shape every Spectrum-fixture mask actually is.
    let fig = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(0, 1)),
            ("type", KiwiValue::Enum("VECTOR".to_owned())),
            ("name", KiwiValue::String("IconMask".to_owned())),
            ("size", vector(24.0, 24.0)),
            ("mask", KiwiValue::Bool(true)),
            ("maskType", KiwiValue::Enum("OUTLINE".to_owned())),
        ],
    )]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    let n = node_named(&doc, "IconMask");
    assert!(
        matches!(n.data, NodeData::Vector(_)),
        "VECTOR maps to a vector node"
    );
    assert!(n.is_mask, "a VECTOR-path node must honor the mask flag");
    assert_eq!(n.mask_type, MaskType::Alpha, "OUTLINE collapses to ALPHA");
    assert_eq!(report.masks_imported, 1);
}

#[test]
fn imports_horizontal_auto_layout_frame_with_padding_align_sizing() {
    use fanta_doc::node::{AxisSizing, CounterAlign, LayoutMode, PrimaryAlign};
    // A HORIZONTAL auto-layout FRAME hugging on the counter axis, centered,
    // space-between, with split padding + spacing — the Action-Bar shape.
    let fig = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(0, 1)),
            ("type", KiwiValue::Enum("FRAME".to_owned())),
            ("name", KiwiValue::String("Action Bar".to_owned())),
            ("size", vector(280.0, 72.0)),
            ("stackMode", KiwiValue::Enum("HORIZONTAL".to_owned())),
            ("stackSpacing", KiwiValue::Float(8.0)),
            ("stackHorizontalPadding", KiwiValue::Float(12.0)),
            ("stackVerticalPadding", KiwiValue::Float(10.0)),
            ("stackPaddingRight", KiwiValue::Float(16.0)),
            (
                "stackPrimaryAlignItems",
                KiwiValue::Enum("SPACE_BETWEEN".to_owned()),
            ),
            (
                "stackCounterAlignItems",
                KiwiValue::Enum("CENTER".to_owned()),
            ),
            ("stackPrimarySizing", KiwiValue::Enum("FIXED".to_owned())),
            (
                "stackCounterSizing",
                KiwiValue::Enum("RESIZE_TO_FIT_WITH_IMPLICIT_SIZE".to_owned()),
            ),
        ],
    )]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    let g = group_named(&doc, "Action Bar");
    let al = g.auto_layout.expect("frame must carry AutoLayout");
    assert_eq!(al.mode, LayoutMode::Horizontal);
    assert_eq!(al.spacing, 8.0);
    // padding [top, right, bottom, left] with the op1 fallback chain:
    // top=vert=10, bottom=vert=10, left=horiz=12, right=stackPaddingRight=16.
    assert_eq!(al.padding, [10.0, 16.0, 10.0, 12.0]);
    assert_eq!(al.primary_align, PrimaryAlign::SpaceBetween);
    assert_eq!(al.counter_align, CounterAlign::Center);
    assert_eq!(al.primary_sizing, AxisSizing::Fixed);
    assert_eq!(al.counter_sizing, AxisSizing::Hug);
    assert!(
        !al.flow_reverse,
        ".fig child order already matches Figma visual flow"
    );
    assert!(
        al.child_layout,
        "explicit stackMode honors stackChild* data"
    );
    assert_eq!(report.auto_layout_horizontal, 1);
    assert_eq!(report.auto_layout_counter_hug, 1);
    assert_eq!(report.auto_layout_primary_hug, 0);
}

#[test]
fn vertical_auto_layout_uses_legacy_padding_and_justify_fallbacks() {
    use fanta_doc::node::{LayoutMode, PrimaryAlign};
    // No per-side padding: `stackPadding` is the uniform fallback for all sides;
    // `stackJustify` is the legacy field read when `stackPrimaryAlignItems` is
    // absent (op1 fallback).
    let fig = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(0, 1)),
            ("type", KiwiValue::Enum("FRAME".to_owned())),
            ("name", KiwiValue::String("Col".to_owned())),
            ("size", vector(100.0, 200.0)),
            ("stackMode", KiwiValue::Enum("VERTICAL".to_owned())),
            ("stackPadding", KiwiValue::Float(6.0)),
            ("stackJustify", KiwiValue::Enum("CENTER".to_owned())),
        ],
    )]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    let al = group_named(&doc, "Col").auto_layout.expect("AutoLayout");
    assert_eq!(al.mode, LayoutMode::Vertical);
    assert_eq!(al.padding, [6.0, 6.0, 6.0, 6.0]);
    assert_eq!(al.primary_align, PrimaryAlign::Center);
    assert_eq!(report.auto_layout_vertical, 1);
}

#[test]
fn non_stack_frame_has_no_auto_layout() {
    // A plain FRAME (no stackMode) must NOT gain an AutoLayout, so plain frames
    // round-trip without one.
    let fig = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(0, 1)),
            ("type", KiwiValue::Enum("FRAME".to_owned())),
            ("name", KiwiValue::String("Plain".to_owned())),
            ("size", vector(50.0, 50.0)),
        ],
    )]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert!(group_named(&doc, "Plain").auto_layout.is_none());
    assert_eq!(
        report.auto_layout_horizontal + report.auto_layout_vertical,
        0
    );
}

#[test]
fn non_stack_frame_with_stale_stack_fields_has_no_auto_layout() {
    // Real .fig files can carry stale stack spacing/padding on free-positioned
    // frames. Figma does not auto-layout those frames unless `stackMode` is an
    // explicit HORIZONTAL/VERTICAL flow; inferring from these fields reflows
    // already-baked compositions.
    let fig = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(0, 1)),
            ("type", KiwiValue::Enum("FRAME".to_owned())),
            ("name", KiwiValue::String("Icon Grid".to_owned())),
            ("size", vector(534.0, 3598.0)),
            ("stackSpacing", KiwiValue::Float(36.0)),
            ("stackPadding", KiwiValue::Float(96.0)),
        ],
    )]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert!(
        group_named(&doc, "Icon Grid").auto_layout.is_none(),
        "stale stack fields without stackMode must not create AutoLayout"
    );
    assert_eq!(
        report.auto_layout_horizontal + report.auto_layout_vertical,
        0
    );
}

#[test]
fn imports_per_child_grow_and_align_self() {
    use fanta_doc::node::CounterAlign;
    // A child carrying grow>0 (FILL primary) + an explicit alignSelf gets a
    // LayoutChild; a plain sibling gets none.
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("FRAME".to_owned())),
                ("name", KiwiValue::String("Bar".to_owned())),
                ("size", vector(200.0, 40.0)),
                ("stackMode", KiwiValue::Enum("HORIZONTAL".to_owned())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("FRAME".to_owned())),
                ("name", KiwiValue::String("Grower".to_owned())),
                ("size", vector(40.0, 40.0)),
                ("stackChildPrimaryGrow", KiwiValue::Float(1.0)),
                ("stackChildAlignSelf", KiwiValue::Enum("STRETCH".to_owned())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 3)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("FRAME".to_owned())),
                ("name", KiwiValue::String("Plain".to_owned())),
                ("size", vector(40.0, 40.0)),
            ],
        ),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    let grower = layout_child_of(&doc, "Grower").expect("grower has layout_child");
    assert_eq!(grower.grow, 1.0);
    assert_eq!(grower.align_self, Some(CounterAlign::Stretch));
    assert!(!grower.absolute);
    assert!(
        layout_child_of(&doc, "Plain").is_none(),
        "plain child carries no layout_child"
    );
    assert_eq!(report.layout_children_grow, 1);
}

#[test]
fn imports_text_auto_resize_into_report_and_node() {
    use fanta_doc::node::TextAutoResize;
    // Three text nodes, one of each autoResize mode; the report tallies them and
    // an auto-width label carries `WidthAndHeight`.
    let mk = |lid: u32, name: &str, ar: Option<&str>| {
        let mut fields = vec![
            ("guid", guid(0, lid)),
            ("type", KiwiValue::Enum("TEXT".to_owned())),
            ("name", KiwiValue::String(name.to_owned())),
            ("size", vector(40.0, 16.0)),
            ("textData", text_data(name)),
        ];
        if let Some(ar) = ar {
            fields.push(("textAutoResize", KiwiValue::Enum(ar.to_owned())));
        }
        o("NodeChange", fields)
    };
    let fig = doc_from(vec![
        mk(1, "Edit", Some("WIDTH_AND_HEIGHT")),
        mk(2, "Body", Some("HEIGHT")),
        mk(3, "Fixed", Some("NONE")),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.text_auto_width, 1);
    assert_eq!(report.text_auto_height, 1);
    assert_eq!(report.text_fixed_box, 1);
    // The auto-width label node carries the enum so a Stage-2 pass can avoid
    // wrapping it.
    let mut found = false;
    for root in doc.scene.roots().to_vec() {
        for id in doc.scene.descendants_of(root) {
            let Some(n) = doc.scene.get(id) else { continue };
            if n.name == "Edit" {
                if let NodeData::Text(t) = &n.data {
                    assert_eq!(t.auto_resize, TextAutoResize::WidthAndHeight);
                    found = true;
                }
            }
        }
    }
    assert!(found, "Edit text node present");
}

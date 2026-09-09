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
                ("type", KiwiValue::Enum("VECTOR".into())),
                ("name", KiwiValue::String("icon".to_owned())),
                ("size", vector(24.0, 18.0)),
                // No fillPaints at all → no visible fill.
                (
                    "strokePaints",
                    KiwiValue::array(vec![solid_paint(0.0, 0.0, 1.0, 1.0)]),
                ),
                ("strokeWeight", KiwiValue::Float(2.0)),
                (
                    "fillGeometry",
                    KiwiValue::array(vec![fig_path(0, "NONZERO")]),
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
                ("type", KiwiValue::Enum("VECTOR".into())),
                ("size", vector(24.0, 18.0)),
                (
                    "fillPaints",
                    KiwiValue::array(vec![solid_paint(1.0, 0.0, 0.0, 1.0)]),
                ),
                (
                    "strokePaints",
                    KiwiValue::array(vec![solid_paint(0.0, 0.0, 1.0, 1.0)]),
                ),
                ("strokeWeight", KiwiValue::Float(2.0)),
                (
                    "fillGeometry",
                    KiwiValue::array(vec![fig_path(0, "NONZERO")]),
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
            ("type", KiwiValue::Enum("SOLID".into())),
            ("color", color(0.0, 0.0, 1.0, 1.0)),
            ("visible", KiwiValue::Bool(false)),
        ],
    );
    let fig = doc_from_with_blobs(
        vec![o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("VECTOR".into())),
                ("size", vector(12.0, 10.0)),
                ("strokePaints", KiwiValue::array(vec![hidden_stroke])),
                ("strokeWeight", KiwiValue::Float(2.0)),
                (
                    "fillGeometry",
                    KiwiValue::array(vec![fig_path(0, "NONZERO")]),
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
                ("type", KiwiValue::Enum("TEXT".into())),
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
            ("type", KiwiValue::Enum("TEXT".into())),
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
            ("type", KiwiValue::Enum("TEXT".into())),
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
            ("type", KiwiValue::Enum("RECTANGLE".into())),
            ("size", vector(100.0, 50.0)),
            (
                "strokePaints",
                KiwiValue::array(vec![solid_paint(0.0, 0.0, 0.0, 1.0)]),
            ),
            ("strokeWeight", KiwiValue::Float(2.0)),
            ("strokeCap", KiwiValue::Enum("ROUND".into())),
            ("strokeJoin", KiwiValue::Enum("BEVEL".into())),
            (
                "dashPattern",
                KiwiValue::array(vec![KiwiValue::Float(4.0), KiwiValue::Float(3.0)]),
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
            ("type", KiwiValue::Enum("RECTANGLE".into())),
            ("size", vector(100.0, 50.0)),
            (
                "strokePaints",
                KiwiValue::array(vec![solid_paint(0.0, 0.0, 0.0, 1.0)]),
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
fn gap_c_miter_limit_field_maps_to_miter_limit() {
    // The real Kiwi field is `NodeChange.miterLimit`, already an SVG-style
    // ratio (e.g. 16 on the Spectrum "Tab Unit" frames) — NOT the fictional
    // `strokeMiterAngle` an earlier version read (that name never occurs in
    // real files, so authored miter limits were silently ignored).
    let rect = o(
        "NodeChange",
        vec![
            ("guid", guid(0, 2)),
            ("parentIndex", parent_index(0, 1)),
            ("type", KiwiValue::Enum("RECTANGLE".into())),
            ("size", vector(100.0, 50.0)),
            (
                "strokePaints",
                KiwiValue::array(vec![solid_paint(0.0, 0.0, 0.0, 1.0)]),
            ),
            ("strokeWeight", KiwiValue::Float(2.0)),
            ("miterLimit", KiwiValue::Float(16.0)),
        ],
    );
    let (doc, _, _) = fig_to_doc(&doc_with_shape(rect)).unwrap();
    let v = first_vector(&doc);
    assert!(
        (v.strokes[0].miter_limit - 16.0).abs() < 1e-6,
        "miterLimit is used as the ratio it already is"
    );
}

#[test]
fn gap_d_text_case_upper_uppercases_content() {
    // `textCase` is a TOP-LEVEL NodeChange field (matching the real .fig schema).
    let fig = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(0, 1)),
            ("type", KiwiValue::Enum("TEXT".into())),
            ("size", vector(100.0, 20.0)),
            ("textData", text_data("Submit")),
            ("textCase", KiwiValue::Enum("UPPER".into())),
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
                ("type", KiwiValue::Enum("TEXT".into())),
                ("size", vector(200.0, 20.0)),
                ("textData", text_data(input)),
                ("textCase", KiwiValue::Enum(case.to_string().into())),
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
            ("type", KiwiValue::Enum("TEXT".into())),
            ("size", vector(100.0, 20.0)),
            (
                "textData",
                o(
                    "TextData",
                    vec![
                        ("characters", KiwiValue::String("submit".to_owned())),
                        ("textCase", KiwiValue::Enum("UPPER".into())),
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
                ("type", KiwiValue::Enum("ROUNDED_RECTANGLE".into())),
                ("name", KiwiValue::String("MaskShape".to_owned())),
                ("size", vector(40.0, 40.0)),
                ("mask", KiwiValue::Bool(true)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("type", KiwiValue::Enum("ROUNDED_RECTANGLE".into())),
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
            ("type", KiwiValue::Enum("ROUNDED_RECTANGLE".into())),
            ("name", KiwiValue::String("Luma".to_owned())),
            ("size", vector(40.0, 40.0)),
            ("mask", KiwiValue::Bool(true)),
            ("maskType", KiwiValue::Enum("LUMINANCE".into())),
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
            ("type", KiwiValue::Enum("ROUNDED_RECTANGLE".into())),
            ("name", KiwiValue::String("Vec".to_owned())),
            ("size", vector(40.0, 40.0)),
            ("mask", KiwiValue::Bool(true)),
            ("maskType", KiwiValue::Enum("VECTOR".into())),
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
            ("type", KiwiValue::Enum("VECTOR".into())),
            ("name", KiwiValue::String("IconMask".to_owned())),
            ("size", vector(24.0, 24.0)),
            ("mask", KiwiValue::Bool(true)),
            ("maskType", KiwiValue::Enum("OUTLINE".into())),
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
            ("type", KiwiValue::Enum("FRAME".into())),
            ("name", KiwiValue::String("Action Bar".to_owned())),
            ("size", vector(280.0, 72.0)),
            ("stackMode", KiwiValue::Enum("HORIZONTAL".into())),
            ("stackSpacing", KiwiValue::Float(8.0)),
            ("stackHorizontalPadding", KiwiValue::Float(12.0)),
            ("stackVerticalPadding", KiwiValue::Float(10.0)),
            ("stackPaddingRight", KiwiValue::Float(16.0)),
            (
                "stackPrimaryAlignItems",
                KiwiValue::Enum("SPACE_BETWEEN".into()),
            ),
            ("stackCounterAlignItems", KiwiValue::Enum("CENTER".into())),
            ("stackPrimarySizing", KiwiValue::Enum("FIXED".into())),
            (
                "stackCounterSizing",
                KiwiValue::Enum("RESIZE_TO_FIT_WITH_IMPLICIT_SIZE".into()),
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
fn nan_counter_spacing_imports_as_auto_gap() {
    // Figma encodes a wrap frame's "Auto" counter gap as a literal NaN
    // `stackCounterSpacing` (UI3 kit icon grids). It must import as the
    // explicit auto flag with a finite 0 spacing — a raw NaN reaching the
    // layout solver poisons every wrapped row's position and the hugged frame
    // height, which made whole icon grids vanish.
    let fig = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(0, 1)),
            ("type", KiwiValue::Enum("FRAME".into())),
            ("name", KiwiValue::String("Icon Grid".to_owned())),
            ("size", vector(928.0, 5728.0)),
            ("stackMode", KiwiValue::Enum("HORIZONTAL".into())),
            ("stackSpacing", KiwiValue::Float(32.0)),
            ("stackCounterSpacing", KiwiValue::Float(f32::NAN)),
            ("stackWrap", KiwiValue::Enum("WRAP".into())),
            ("stackPrimarySizing", KiwiValue::Enum("FIXED".into())),
            (
                "stackCounterSizing",
                KiwiValue::Enum("RESIZE_TO_FIT_WITH_IMPLICIT_SIZE".into()),
            ),
        ],
    )]);
    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    let al = group_named(&doc, "Icon Grid")
        .auto_layout
        .expect("AutoLayout");
    assert!(al.wrap);
    assert!(al.counter_auto_spacing, "NaN counter gap means Auto");
    assert_eq!(al.counter_spacing, 0.0, "no NaN may survive the import");
    assert_eq!(al.spacing, 32.0);
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
            ("type", KiwiValue::Enum("FRAME".into())),
            ("name", KiwiValue::String("Col".to_owned())),
            ("size", vector(100.0, 200.0)),
            ("stackMode", KiwiValue::Enum("VERTICAL".into())),
            ("stackPadding", KiwiValue::Float(6.0)),
            ("stackJustify", KiwiValue::Enum("CENTER".into())),
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
            ("type", KiwiValue::Enum("FRAME".into())),
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
            ("type", KiwiValue::Enum("FRAME".into())),
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
                ("type", KiwiValue::Enum("FRAME".into())),
                ("name", KiwiValue::String("Bar".to_owned())),
                ("size", vector(200.0, 40.0)),
                ("stackMode", KiwiValue::Enum("HORIZONTAL".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("FRAME".into())),
                ("name", KiwiValue::String("Grower".to_owned())),
                ("size", vector(40.0, 40.0)),
                ("stackChildPrimaryGrow", KiwiValue::Float(1.0)),
                ("stackChildAlignSelf", KiwiValue::Enum("STRETCH".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 3)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("FRAME".into())),
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
fn imports_api_named_per_child_layout_fields() {
    use fanta_doc::node::CounterAlign;

    let fig = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(0, 1)),
            ("type", KiwiValue::Enum("FRAME".into())),
            ("name", KiwiValue::String("Tip".to_owned())),
            ("size", vector(8.0, 4.0)),
            ("layoutGrow", KiwiValue::Float(1.0)),
            ("layoutPositioning", KiwiValue::Enum("ABSOLUTE".into())),
            ("layoutAlignSelf", KiwiValue::Enum("CENTER".into())),
        ],
    )]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    let child = layout_child_of(&doc, "Tip").expect("child has layout data");
    assert_eq!(child.grow, 1.0);
    assert!(child.absolute);
    assert_eq!(child.align_self, Some(CounterAlign::Center));
    assert_eq!(report.layout_children_grow, 1);
    assert_eq!(report.layout_children_absolute, 1);
}

#[test]
fn imports_text_auto_resize_into_report_and_node() {
    use fanta_doc::node::TextAutoResize;
    // Three text nodes, one of each autoResize mode; the report tallies them and
    // an auto-width label carries `WidthAndHeight`.
    let mk = |lid: u32, name: &str, ar: Option<&str>| {
        let mut fields = vec![
            ("guid", guid(0, lid)),
            ("type", KiwiValue::Enum("TEXT".into())),
            ("name", KiwiValue::String(name.to_owned())),
            ("size", vector(40.0, 16.0)),
            ("textData", text_data(name)),
        ];
        if let Some(ar) = ar {
            fields.push(("textAutoResize", KiwiValue::Enum(ar.into())));
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

// =============================================================================
// Scroll semantics import (D3/F2): scrollDirection, scrollBehavior, scrollOffset
// =============================================================================

#[test]
fn pr1_frame_scroll_direction_and_offset_import() {
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 0)),
                ("type", KiwiValue::Enum("DOCUMENT".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("parentIndex", parent_index(0, 0)),
                ("type", KiwiValue::Enum("CANVAS".into())),
                ("name", KiwiValue::String("Page 1".to_owned())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("FRAME".into())),
                ("name", KiwiValue::String("Feed".to_owned())),
                ("size", vector(400.0, 300.0)),
                (
                    "scrollDirection",
                    KiwiValue::Enum("VERTICAL_SCROLLING".into()),
                ),
                ("scrollOffset", vector(0.0, 120.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 3)),
                ("parentIndex", parent_index(0, 2)),
                ("type", KiwiValue::Enum("FRAME".into())),
                ("name", KiwiValue::String("Header".to_owned())),
                ("size", vector(400.0, 60.0)),
                (
                    "scrollBehavior",
                    KiwiValue::Enum("FIXED_WHEN_CHILD_OF_SCROLLING_FRAME".into()),
                ),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 4)),
                ("parentIndex", parent_index(0, 2)),
                ("type", KiwiValue::Enum("FRAME".into())),
                ("name", KiwiValue::String("Section title".to_owned())),
                ("size", vector(400.0, 40.0)),
                ("scrollBehavior", KiwiValue::Enum("STICKY_SCROLLS".into())),
            ],
        ),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();

    let canvas = doc.scene.roots()[0];
    let feed = doc.scene.children_of(Some(canvas))[0];
    match &doc.scene.get(feed).unwrap().data {
        NodeData::Group(group) => {
            assert_eq!(
                group.scroll_direction,
                Some(fanta_doc::ScrollDirection::Vertical)
            );
            assert_eq!(group.scroll_offset, Some([0.0, 120.0]));
            assert_eq!(
                group.effective_scroll_direction(),
                fanta_doc::ScrollDirection::Vertical
            );
        }
        other => panic!("feed should be a group, got {other:?}"),
    }

    let children = doc.scene.children_of(Some(feed)).to_vec();
    let by_name = |name: &str| {
        children
            .iter()
            .find(|id| doc.scene.get(**id).unwrap().name == name)
            .copied()
            .unwrap()
    };
    assert_eq!(
        doc.scene.get(by_name("Header")).unwrap().scroll_behavior,
        fanta_doc::ScrollBehavior::Fixed
    );
    assert_eq!(
        doc.scene
            .get(by_name("Section title"))
            .unwrap()
            .scroll_behavior,
        fanta_doc::ScrollBehavior::Sticky
    );

    assert_eq!(report.scroll_frames, 1);
    assert_eq!(report.scroll_pinned_children, 2);
}

#[test]
fn frame_without_scroll_fields_imports_as_non_scrolling() {
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 0)),
                ("type", KiwiValue::Enum("DOCUMENT".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("parentIndex", parent_index(0, 0)),
                ("type", KiwiValue::Enum("CANVAS".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("FRAME".into())),
                ("size", vector(400.0, 300.0)),
            ],
        ),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    let canvas = doc.scene.roots()[0];
    let frame = doc.scene.children_of(Some(canvas))[0];
    match &doc.scene.get(frame).unwrap().data {
        NodeData::Group(group) => {
            // Authored axes say NO even though the legacy bool stays true for
            // older readers — effective wins.
            assert_eq!(
                group.scroll_direction,
                Some(fanta_doc::ScrollDirection::None)
            );
            assert!(group.scrollable);
            assert_eq!(
                group.effective_scroll_direction(),
                fanta_doc::ScrollDirection::None
            );
        }
        other => panic!("frame should be a group, got {other:?}"),
    }
    assert_eq!(report.scroll_frames, 0);
}

// =============================================================================
// Flows + presentation device import (D5/F4)
// =============================================================================

#[test]
fn pr4_named_flows_and_prototype_device_import() {
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 0)),
                ("type", KiwiValue::Enum("DOCUMENT".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("parentIndex", parent_index(0, 0)),
                ("type", KiwiValue::Enum("CANVAS".into())),
                (
                    "flowStartingPoints",
                    KiwiValue::array(vec![
                        o(
                            "FlowStartingPoint",
                            vec![
                                ("nodeID", guid(0, 2)),
                                ("name", KiwiValue::String("Onboarding".to_owned())),
                            ],
                        ),
                        o(
                            "FlowStartingPoint",
                            vec![
                                ("nodeID", guid(0, 3)),
                                ("name", KiwiValue::String("Checkout".to_owned())),
                            ],
                        ),
                    ]),
                ),
                (
                    "prototypeDevice",
                    o(
                        "PrototypeDevice",
                        vec![
                            ("size", vector(393.0, 852.0)),
                            (
                                "presetIdentifier",
                                KiwiValue::String("IPHONE_16_PRO".to_owned()),
                            ),
                            ("rotation", KiwiValue::Enum("NONE".into())),
                        ],
                    ),
                ),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("FRAME".into())),
                ("name", KiwiValue::String("Welcome".to_owned())),
                ("size", vector(393.0, 852.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 3)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("FRAME".into())),
                ("name", KiwiValue::String("Cart".to_owned())),
                ("size", vector(393.0, 852.0)),
            ],
        ),
    ]);
    let (doc, _report, _) = fig_to_doc(&fig).unwrap();

    assert_eq!(doc.flows.len(), 2, "both named flows import");
    assert_eq!(doc.flows[0].name, "Onboarding");
    assert_eq!(doc.flows[1].name, "Checkout");
    // No document-level start: the first flow backs flow_start.
    assert_eq!(doc.flow_start, Some(doc.flows[0].start));

    let device = doc.presentation.expect("prototype device imports");
    assert_eq!(device.device_size, Some([393.0, 852.0]));
    assert_eq!(device.preset.as_deref(), Some("IPHONE_16_PRO"));
    assert!(!device.landscape);
}

// =============================================================================
// GLASS effect approximation (parity doc RE-4)
// =============================================================================

#[test]
fn glass_effect_approximates_as_background_blur_and_is_counted() {
    let glass = o(
        "Effect",
        vec![
            ("type", KiwiValue::Enum("GLASS".into())),
            ("radius", KiwiValue::Float(38.0)),
            ("refractionRadius", KiwiValue::Float(20.0)),
            ("visible", KiwiValue::Bool(true)),
        ],
    );
    let noise = o(
        "Effect",
        vec![
            ("type", KiwiValue::Enum("NOISE".into())),
            ("visible", KiwiValue::Bool(true)),
        ],
    );
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 0)),
                ("type", KiwiValue::Enum("DOCUMENT".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("parentIndex", parent_index(0, 0)),
                ("type", KiwiValue::Enum("CANVAS".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("FRAME".into())),
                ("name", KiwiValue::String("Panel".to_owned())),
                ("size", vector(200.0, 100.0)),
                ("effects", KiwiValue::array(vec![glass, noise])),
            ],
        ),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    let canvas = doc.scene.roots()[0];
    let panel = doc.scene.children_of(Some(canvas))[0];
    let node = doc.scene.get(panel).unwrap();
    assert_eq!(node.blurs.len(), 1, "GLASS lands as one blur");
    assert_eq!(node.blurs[0].kind, fanta_doc::BlurKind::Background);
    assert_eq!(node.blurs[0].radius, 38.0);
    assert_eq!(report.effects_glass_approximated, 1);
    assert_eq!(
        report.effects_dropped_unknown, 1,
        "NOISE counted as dropped"
    );
}

// =============================================================================
// Interactive components: SWAP_STATE is a variant swap, not a navigation
// =============================================================================

#[test]
fn swap_state_navigation_imports_as_update_variant() {
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 0)),
                ("type", KiwiValue::Enum("DOCUMENT".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("parentIndex", parent_index(0, 0)),
                ("type", KiwiValue::Enum("CANVAS".into())),
            ],
        ),
        // A two-variant component set: Default + Hover masters.
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 10)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String("State=Default".to_owned())),
                ("size", vector(100.0, 40.0)),
                (
                    "prototypeInteractions",
                    KiwiValue::array(vec![o(
                        "PrototypeInteraction",
                        vec![
                            (
                                "event",
                                o(
                                    "PrototypeEvent",
                                    vec![("interactionType", KiwiValue::Enum("ON_HOVER".into()))],
                                ),
                            ),
                            (
                                "actions",
                                KiwiValue::array(vec![o(
                                    "PrototypeAction",
                                    vec![
                                        ("navigationType", KiwiValue::Enum("SWAP_STATE".into())),
                                        ("transitionNodeID", guid(0, 11)),
                                        ("transitionType", KiwiValue::Enum("SMART_ANIMATE".into())),
                                        ("transitionDuration", KiwiValue::Float(0.3)),
                                    ],
                                )]),
                            ),
                        ],
                    )]),
                ),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 11)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String("State=Hover".to_owned())),
                ("size", vector(100.0, 40.0)),
            ],
        ),
    ]);
    let (doc, _report, _) = fig_to_doc(&fig).unwrap();

    // Find the Default master's reaction.
    let reaction = doc
        .scene
        .roots()
        .iter()
        .flat_map(|root| std::iter::once(*root).chain(doc.scene.descendants_of(*root)))
        .filter_map(|id| doc.scene.get(id))
        .flat_map(|node| node.reactions.iter())
        .next()
        .expect("hover reaction imported");
    match &reaction.action {
        fanta_doc::Action::UpdateVariant { variant, .. } => {
            assert_eq!(variant, "State=Hover", "targets the hover variant");
        }
        other => panic!("SWAP_STATE must import as UpdateVariant, got {other:?}"),
    }
}

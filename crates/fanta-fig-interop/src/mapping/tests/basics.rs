//! Original mapping behavior: frames, ellipses, text, pages, transforms.

use super::*;

// =============================================================================
// Original mapping behavior (preserved)
// =============================================================================

#[test]
fn frame_with_rectangle_maps_to_group_with_vector_child() {
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 0)),
                ("type", KiwiValue::Enum("DOCUMENT".to_owned())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("parentIndex", parent_index(0, 0)),
                ("type", KiwiValue::Enum("CANVAS".to_owned())),
                ("name", KiwiValue::String("Page 1".to_owned())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("FRAME".to_owned())),
                ("name", KiwiValue::String("Frame 1".to_owned())),
                ("size", vector(400.0, 300.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 3)),
                ("parentIndex", parent_index(0, 2)),
                ("type", KiwiValue::Enum("RECTANGLE".to_owned())),
                ("name", KiwiValue::String("Rect".to_owned())),
                ("size", vector(100.0, 50.0)),
                (
                    "fillPaints",
                    KiwiValue::Array(vec![solid_paint(1.0, 0.0, 0.0, 1.0)]),
                ),
            ],
        ),
    ]);

    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.mapped, 3);
    assert_eq!(report.skipped(), 0);

    let roots = doc.scene.roots();
    assert_eq!(roots.len(), 1);
    let canvas_id = roots[0];
    let canvas = doc.scene.get(canvas_id).unwrap();
    assert_eq!(canvas.name, "Page 1");
    assert!(canvas.is_group());

    let canvas_children = doc.scene.children_of(Some(canvas_id));
    assert_eq!(canvas_children.len(), 1);
    let frame_id = canvas_children[0];
    match &doc.scene.get(frame_id).unwrap().data {
        NodeData::Group(g) => assert_eq!(g.clip_size, Some([400.0, 300.0])),
        _ => panic!("frame should be a group"),
    }

    let rect = doc
        .scene
        .get(doc.scene.children_of(Some(frame_id))[0])
        .unwrap();
    match &rect.data {
        NodeData::Vector(v) => {
            assert_eq!(v.fills[0], Fill::solid(Color::rgba(255, 0, 0, 255)));
            let b = v.path.rough_bounds().unwrap();
            assert_eq!((b.width(), b.height()), (100.0, 50.0));
        }
        _ => panic!("rectangle should be a vector"),
    }
    doc.scene.validate().unwrap();
}

#[test]
fn frame_import_preserves_stacked_background_fills() {
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 0)),
                ("type", KiwiValue::Enum("DOCUMENT".to_owned())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("parentIndex", parent_index(0, 0)),
                ("type", KiwiValue::Enum("CANVAS".to_owned())),
                ("name", KiwiValue::String("Page 1".to_owned())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("FRAME".to_owned())),
                ("name", KiwiValue::String("Stacked fills".to_owned())),
                ("size", vector(100.0, 80.0)),
                (
                    "fillPaints",
                    KiwiValue::Array(vec![
                        solid_paint(1.0, 0.0, 0.0, 1.0),
                        solid_paint(0.0, 0.0, 1.0, 1.0),
                    ]),
                ),
            ],
        ),
    ]);

    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    let page = doc.scene.roots()[0];
    let frame = doc.scene.get(doc.scene.children_of(Some(page))[0]).unwrap();
    let NodeData::Group(g) = &frame.data else {
        panic!("frame should import as group");
    };
    assert_eq!(g.background, Some(Fill::solid(Color::rgba(255, 0, 0, 255))));
    assert_eq!(g.background_fills.len(), 1);
    assert_eq!(
        g.background_fills[0],
        Fill::solid(Color::rgba(0, 0, 255, 255))
    );
}

/// Build a single FRAME (parented to a CANVAS) and return its imported
/// `clip_size` plus the `frames_clip_disabled` report counter. `clip_flag` lets a
/// test set `frameMaskDisabled` to a given value, or leave it absent (`None`).
fn frame_clip(clip_flag: Option<bool>) -> (Option<[f64; 2]>, Option<bool>, usize) {
    let mut frame_fields = vec![
        ("guid", guid(0, 2)),
        ("parentIndex", parent_index(0, 1)),
        ("type", KiwiValue::Enum("FRAME".to_owned())),
        ("name", KiwiValue::String("Frame 1".to_owned())),
        ("size", vector(400.0, 300.0)),
    ];
    if let Some(b) = clip_flag {
        frame_fields.push(("frameMaskDisabled", KiwiValue::Bool(b)));
    }
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 0)),
                ("type", KiwiValue::Enum("DOCUMENT".to_owned())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("parentIndex", parent_index(0, 0)),
                ("type", KiwiValue::Enum("CANVAS".to_owned())),
                ("name", KiwiValue::String("Page 1".to_owned())),
            ],
        ),
        o("NodeChange", frame_fields),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    let canvas_id = doc.scene.roots()[0];
    let frame_id = doc.scene.children_of(Some(canvas_id))[0];
    let frame = doc.scene.get(frame_id).unwrap();
    let clip = match &frame.data {
        NodeData::Group(g) => g.clip_size,
        _ => panic!("frame should be a group"),
    };
    let clip_content = frame.meta.get("clip_content").and_then(|v| v.as_bool());
    (clip, clip_content, report.frames_clip_disabled)
}

#[test]
fn frame_clips_by_default_and_honors_frame_mask_disabled() {
    // Figma stores "Clip content" as `frameMaskDisabled`: true => clip OFF;
    // absent/false => clip ON (the default). The importer must honor it so a
    // frame that intentionally lets content bleed past its edge is not cropped.

    // Absent => clips (the default, the box drives `clip_size`).
    let (clip, clip_content, disabled) = frame_clip(None);
    assert_eq!(clip, Some([400.0, 300.0]), "absent flag must clip");
    assert_eq!(clip_content, None, "absent flag uses default clip behavior");
    assert_eq!(disabled, 0, "absent flag is not a clip-disabled frame");

    // Explicit false => clips.
    let (clip, clip_content, disabled) = frame_clip(Some(false));
    assert_eq!(
        clip,
        Some([400.0, 300.0]),
        "frameMaskDisabled=false must clip"
    );
    assert_eq!(
        clip_content, None,
        "frameMaskDisabled=false uses default clip behavior"
    );
    assert_eq!(disabled, 0);

    // True => clip OFF, but the frame's own box remains for background/border.
    let (clip, clip_content, disabled) = frame_clip(Some(true));
    assert_eq!(
        clip,
        Some([400.0, 300.0]),
        "frameMaskDisabled=true must keep the frame box"
    );
    assert_eq!(
        clip_content,
        Some(false),
        "frameMaskDisabled=true must disable only the content clip"
    );
    assert_eq!(
        disabled, 1,
        "a clip-disabled frame is counted in the report"
    );
}

#[test]
fn ellipse_maps_to_vector_ellipse() {
    let fig = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(0, 1)),
            ("type", KiwiValue::Enum("ELLIPSE".to_owned())),
            ("size", vector(80.0, 40.0)),
        ],
    )]);
    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    let id = doc.scene.roots()[0];
    match &doc.scene.get(id).unwrap().data {
        NodeData::Vector(v) => assert_eq!(v.path.segments.len(), 6),
        _ => panic!("ellipse should be a vector"),
    }
}

#[test]
fn rounded_rectangle_carries_corner_radius() {
    let fig = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(0, 1)),
            ("type", KiwiValue::Enum("ROUNDED_RECTANGLE".to_owned())),
            ("size", vector(50.0, 50.0)),
            ("cornerRadius", KiwiValue::Float(8.0)),
        ],
    )]);
    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    let id = doc.scene.roots()[0];
    match &doc.scene.get(id).unwrap().data {
        NodeData::Vector(v) => assert_eq!(v.corner_radius, Some(8.0)),
        _ => panic!("should be a vector"),
    }
}

#[test]
fn transform_maps_matrix_components_in_svg_order() {
    let matrix = o(
        "Matrix",
        vec![
            ("m00", KiwiValue::Float(2.0)),
            ("m01", KiwiValue::Float(0.0)),
            ("m02", KiwiValue::Float(120.0)),
            ("m10", KiwiValue::Float(0.0)),
            ("m11", KiwiValue::Float(1.0)),
            ("m12", KiwiValue::Float(30.0)),
        ],
    );
    let fig = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(0, 1)),
            ("type", KiwiValue::Enum("RECTANGLE".to_owned())),
            ("size", vector(10.0, 10.0)),
            ("transform", matrix),
        ],
    )]);
    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    let id = doc.scene.roots()[0];
    let comps = doc.scene.get(id).unwrap().transform.to_components();
    assert_eq!(comps, [2.0, 0.0, 0.0, 1.0, 120.0, 30.0]);
}

#[test]
fn child_of_skipped_node_reparents_to_nearest_recognized_ancestor() {
    // FRAME -> SLICE(skipped) -> RECTANGLE; the rect attaches to the FRAME.
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("FRAME".to_owned())),
                ("size", vector(200.0, 200.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("DOCUMENT".to_owned())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 3)),
                ("parentIndex", parent_index(0, 2)),
                ("type", KiwiValue::Enum("RECTANGLE".to_owned())),
                ("size", vector(10.0, 10.0)),
            ],
        ),
    ]);
    let (doc, _report, _) = fig_to_doc(&fig).unwrap();
    let roots = doc.scene.roots();
    assert_eq!(roots.len(), 1);
    let frame_id = roots[0];
    let kids = doc.scene.children_of(Some(frame_id));
    assert_eq!(kids.len(), 1);
    assert!(matches!(
        doc.scene.get(kids[0]).unwrap().data,
        NodeData::Vector(_)
    ));
    doc.scene.validate().unwrap();
}

#[test]
fn text_node_maps_to_text_with_content_size_weight_and_align() {
    let fig = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(0, 1)),
            ("type", KiwiValue::Enum("TEXT".to_owned())),
            ("name", KiwiValue::String("Heading".to_owned())),
            ("size", vector(200.0, 40.0)),
            ("textData", text_data("Hello Fantaisa")),
            ("fontSize", KiwiValue::Float(24.0)),
            ("fontName", font_name("Inter", "Bold Italic")),
            ("textAlignHorizontal", KiwiValue::Enum("CENTER".to_owned())),
            (
                "fillPaints",
                KiwiValue::Array(vec![solid_paint(1.0, 0.0, 0.0, 1.0)]),
            ),
        ],
    )]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.mapped, 1);
    let node = doc.scene.get(doc.scene.roots()[0]).unwrap();
    assert_eq!(node.name, "Heading");
    match &node.data {
        NodeData::Text(t) => {
            assert_eq!(t.content, "Hello Fantaisa");
            assert_eq!(t.align, TextAlign::Center);
            assert_eq!(t.style.size_px, 24.0);
            assert_eq!(t.style.font_family, "Inter");
            assert_eq!(t.style.weight, 700);
            assert!(t.style.italic);
            assert_eq!(t.style.color, Color::rgba(255, 0, 0, 255));
        }
        other => panic!("TEXT should map to NodeData::Text, got {other:?}"),
    }
}

#[test]
fn text_vertical_align_maps_center_and_bottom() {
    let make = |valign: &str| {
        let fig = doc_from(vec![o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("TEXT".to_owned())),
                ("size", vector(200.0, 40.0)),
                ("textData", text_data("Btn")),
                ("textAlignVertical", KiwiValue::Enum(valign.to_owned())),
            ],
        )]);
        let (doc, _, _) = fig_to_doc(&fig).unwrap();
        match &doc.scene.get(doc.scene.roots()[0]).unwrap().data {
            NodeData::Text(t) => t.vertical_align,
            other => panic!("expected text, got {other:?}"),
        }
    };
    assert_eq!(make("CENTER"), fanta_doc::VAlign::Center);
    assert_eq!(make("BOTTOM"), fanta_doc::VAlign::Bottom);
    assert_eq!(make("TOP"), fanta_doc::VAlign::Top);
    // Absent field defaults to Top.
    let fig = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(0, 1)),
            ("type", KiwiValue::Enum("TEXT".to_owned())),
            ("size", vector(200.0, 40.0)),
            ("textData", text_data("Btn")),
        ],
    )]);
    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    match &doc.scene.get(doc.scene.roots()[0]).unwrap().data {
        NodeData::Text(t) => assert_eq!(t.vertical_align, fanta_doc::VAlign::Top),
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn text_line_height_and_letter_spacing_units_are_converted() {
    let fig = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(0, 1)),
            ("type", KiwiValue::Enum("TEXT".to_owned())),
            ("size", vector(100.0, 100.0)),
            ("textData", text_data("spaced")),
            ("fontSize", KiwiValue::Float(20.0)),
            ("lineHeight", number(150.0, "PERCENT")),
            ("letterSpacing", number(2.0, "PIXELS")),
        ],
    )]);
    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    match &doc.scene.get(doc.scene.roots()[0]).unwrap().data {
        NodeData::Text(t) => {
            assert_eq!(t.style.line_height, 1.5);
            assert_eq!(t.style.letter_spacing, 2.0);
        }
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn canvases_are_registered_as_pages_in_document_order() {
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 0)),
                ("type", KiwiValue::Enum("DOCUMENT".to_owned())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("parentIndex", parent_index(0, 0)),
                ("type", KiwiValue::Enum("CANVAS".to_owned())),
                ("name", KiwiValue::String("Cover".to_owned())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("FRAME".to_owned())),
                ("size", vector(400.0, 300.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 3)),
                ("parentIndex", parent_index(0, 0)),
                ("type", KiwiValue::Enum("CANVAS".to_owned())),
                ("name", KiwiValue::String("Empty".to_owned())),
            ],
        ),
    ]);
    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    let pages = doc.pages();
    assert_eq!(pages.len(), 2);
    assert_eq!(doc.page_name(pages[0]), Some("Cover"));
    assert_eq!(doc.active_page(), Some(pages[0]));
}

#[test]
fn invisible_and_gradient_paints_are_not_taken_as_solid_fill() {
    let invisible_solid = o(
        "Paint",
        vec![
            ("type", KiwiValue::Enum("SOLID".to_owned())),
            ("visible", KiwiValue::Bool(false)),
            ("color", color(0.0, 1.0, 0.0, 1.0)),
        ],
    );
    let gradient = o(
        "Paint",
        vec![("type", KiwiValue::Enum("GRADIENT_LINEAR".to_owned()))],
    );
    let fig = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(0, 1)),
            ("type", KiwiValue::Enum("RECTANGLE".to_owned())),
            ("size", vector(10.0, 10.0)),
            (
                "fillPaints",
                KiwiValue::Array(vec![invisible_solid, gradient]),
            ),
        ],
    )]);
    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    match &doc.scene.get(doc.scene.roots()[0]).unwrap().data {
        NodeData::Vector(v) => assert!(v.fills.is_empty()),
        _ => panic!("should be a vector"),
    }
}

#[test]
fn document_without_node_changes_is_a_mapping_error() {
    let fig = FigDocument {
        version: 0,
        schema: schema(),
        root: o("Message", vec![]),
        root_type_name: "Message".to_owned(),
        blobs: Vec::new(),
        images: std::collections::HashMap::new(),
    };
    assert!(matches!(fig_to_doc(&fig), Err(FigError::Mapping(_))));
}

#[test]
fn end_to_end_through_the_container() {
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 0)),
                ("type", KiwiValue::Enum("DOCUMENT".to_owned())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("parentIndex", parent_index(0, 0)),
                ("type", KiwiValue::Enum("RECTANGLE".to_owned())),
                ("name", KiwiValue::String("Solo".to_owned())),
                ("size", vector(64.0, 64.0)),
                (
                    "fillPaints",
                    KiwiValue::Array(vec![solid_paint(0.0, 0.0, 0.0, 1.0)]),
                ),
            ],
        ),
    ]);
    let bytes = crate::fig::write_fig(&fig).unwrap();
    let reread = crate::fig::read_fig(&bytes).unwrap();
    let (doc, report, _) = fig_to_doc(&reread).unwrap();
    assert_eq!(report.mapped, 1);
    assert_eq!(doc.scene.get(doc.scene.roots()[0]).unwrap().name, "Solo");
}

#[test]
fn built_node_meta_carries_figma_id_in_sid_lid_form() {
    // A node built from a `NodeChange` whose guid is {sessionID:7, localID:42}
    // must stamp `meta.figma_id == "7:42"` (Figma public node-id form) alongside
    // `figma_type`, so the fidelity farm can correlate a rendered frame back to
    // its source Figma node.
    let fig = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(7, 42)),
            ("type", KiwiValue::Enum("RECTANGLE".to_owned())),
            ("size", vector(10.0, 10.0)),
        ],
    )]);
    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    let node = doc.scene.get(doc.scene.roots()[0]).unwrap();
    assert_eq!(
        node.meta.get("figma_id").and_then(|v| v.as_str()),
        Some("7:42"),
        "meta should carry figma_id in sid:lid form"
    );
    assert_eq!(
        node.meta.get("figma_type").and_then(|v| v.as_str()),
        Some("RECTANGLE"),
        "figma_type must be preserved alongside figma_id"
    );
}
